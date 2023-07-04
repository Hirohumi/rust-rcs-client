// Copyright 2023 宋昊文
//
// Licensed under the Apache License, Version 2.0 (the "License");
// you may not use this file except in compliance with the License.
// You may obtain a copy of the License at
//
//     http://www.apache.org/licenses/LICENSE-2.0
//
// Unless required by applicable law or agreed to in writing, software
// distributed under the License is distributed on an "AS IS" BASIS,
// WITHOUT WARRANTIES OR CONDITIONS OF ANY KIND, either express or implied.
// See the License for the specific language governing permissions and
// limitations under the License.

use std::{
    fmt::Display,
    fs::{File, OpenOptions},
    io::{self, Seek, Write},
    pin::Pin,
    sync::Arc,
    task::{Context, Poll},
};

use futures::{future::BoxFuture, io::copy_buf, AsyncWrite, FutureExt};
use rust_rcs_core::{
    ffi::log::platform_log,
    http::{
        request::{Request, GET},
        HttpClient,
    },
    internet::{header, Header},
    security::{
        authentication::digest::DigestAnswerParams,
        gba::{self, GbaContext},
        SecurityContext,
    },
};
use url::Url;

const LOG_TAG: &str = "fthttp";

pub enum FileDownloadError {
    Http(u16, String),
    IO,
    MalformedHost,
    NetworkIO,
}

impl FileDownloadError {
    pub fn error_code(&self) -> u16 {
        match &self {
            FileDownloadError::Http(status_code, _) => *status_code,
            FileDownloadError::IO => 0,
            FileDownloadError::MalformedHost => 0,
            FileDownloadError::NetworkIO => 0,
        }
    }

    pub fn error_string(&self) -> String {
        match &self {
            FileDownloadError::Http(_, reason_phrase) => String::from(reason_phrase),
            FileDownloadError::IO => String::from("IO"),
            FileDownloadError::MalformedHost => String::from("MalformedHost"),
            FileDownloadError::NetworkIO => String::from("NetworkIO"),
        }
    }
}

impl Display for FileDownloadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            FileDownloadError::Http(status_code, reason_phrase) => {
                f.write_fmt(format_args!("Http {} {}", status_code, reason_phrase))
            }
            FileDownloadError::IO => f.write_str("IO"),
            FileDownloadError::MalformedHost => f.write_str("MalformedHost"),
            FileDownloadError::NetworkIO => f.write_str("NetworkIO"),
        }
    }
}

struct FileOutput {
    f: File,
}

impl AsyncWrite for FileOutput {
    fn poll_write(
        self: Pin<&mut Self>,
        _cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<io::Result<usize>> {
        let p = self.get_mut();
        match p.f.write(buf) {
            Ok(i) => Poll::Ready(Ok(i)),
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => Poll::Pending,
                _ => Poll::Ready(Err(e)),
            },
        }
    }

    fn poll_flush(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let p = self.get_mut();
        match p.f.flush() {
            Ok(()) => Poll::Ready(Ok(())),
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => Poll::Pending,
                _ => Poll::Ready(Err(e)),
            },
        }
    }

    fn poll_close(self: Pin<&mut Self>, _cx: &mut Context<'_>) -> Poll<io::Result<()>> {
        let p = self.get_mut();
        match p.f.sync_all() {
            Ok(()) => Poll::Ready(Ok(())),
            Err(e) => match e.kind() {
                io::ErrorKind::WouldBlock => Poll::Pending,
                _ => Poll::Ready(Err(e)),
            },
        }
    }
}

async fn download_file_inner(
    file_uri: &str,
    download_path: &str,
    start: usize,
    total: usize,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    digest_answer: Option<&DigestAnswerParams>,
) -> Result<(), FileDownloadError> {
    if let Ok(url) = Url::parse(file_uri) {
        if let Ok(conn) = http_client.connect(&url, false).await {
            let host = url.host_str().unwrap();

            let mut req = Request::new_with_default_headers(GET, host, url.path(), url.query());

            if let Some(msisdn) = msisdn {
                req.headers.push(Header::new(
                    b"X-3GPP-Intended-Identity",
                    format!("tel:{}", msisdn),
                ));
            }

            let preloaded_answer = match digest_answer {
                Some(_) => None,
                None => {
                    platform_log(LOG_TAG, "using stored authorization info");
                    security_context.preload_auth(gba_context, host, conn.cipher_id(), GET, None)
                }
            };

            let digest_answer = match digest_answer {
                Some(digest_answer) => Some(digest_answer),
                None => match &preloaded_answer {
                    Some(preloaded_answer) => {
                        platform_log(LOG_TAG, "using preloaded digest answer");
                        Some(preloaded_answer)
                    }
                    None => None,
                },
            };

            if let Some(digest_answer) = digest_answer {
                if let Ok(authorization) = digest_answer.make_authorization_header(
                    match &digest_answer.challenge {
                        Some(challenge) => Some(&challenge.algorithm),
                        None => None,
                    },
                    false,
                    false,
                ) {
                    if let Ok(authorization) = String::from_utf8(authorization) {
                        req.headers
                            .push(Header::new(b"Authorization", String::from(authorization)));
                    }
                }
            }

            if start > 0 {
                req.headers.push(Header::new(
                    b"Range",
                    format!("bytes={}-{}", start, total - 1),
                ));
            }

            if let Ok((resp, resp_stream)) = conn.send(req).await {
                platform_log(
                    LOG_TAG,
                    format!(
                        "download_file_inner resp.status_code = {}",
                        resp.status_code
                    ),
                );

                if resp.status_code == 200 {
                    if let Some(authentication_info_header) =
                        header::search(&resp.headers, b"Authentication-Info", false)
                    {
                        if let Some(digest_answer) = digest_answer {
                            if let Some(challenge) = &digest_answer.challenge {
                                security_context.update_auth_info(
                                    authentication_info_header,
                                    host,
                                    b"\"",
                                    challenge,
                                    false,
                                );
                            }
                        }
                    }

                    if let Some(mut resp_stream) = resp_stream {
                        let mut f = if start == 0 {
                            match OpenOptions::new()
                                .write(true)
                                .create(true)
                                .open(download_path)
                            {
                                Ok(f) => f,
                                Err(e) => {
                                    platform_log(LOG_TAG, format!("file create error: {}", e));
                                    return Err(FileDownloadError::IO);
                                }
                            }
                        } else {
                            match OpenOptions::new()
                                .write(true)
                                .append(true)
                                .open(download_path)
                            {
                                Ok(f) => f,
                                Err(e) => {
                                    platform_log(LOG_TAG, format!("file open error: {}", e));
                                    match OpenOptions::new()
                                        .write(true)
                                        .create(true)
                                        .open(download_path)
                                    {
                                        Ok(f) => f,
                                        Err(e) => {
                                            platform_log(
                                                LOG_TAG,
                                                format!("file create error: {}", e),
                                            );
                                            return Err(FileDownloadError::IO);
                                        }
                                    }
                                }
                            }
                        };

                        loop {
                            match f.seek(io::SeekFrom::Current(0)) {
                                Ok(i) => {
                                    if let Ok(i) = usize::try_from(i) {
                                        if i == start {
                                            break;
                                        }
                                    }
                                    platform_log(
                                        LOG_TAG,
                                        "file range is inconsistent with provided value",
                                    );
                                }
                                Err(e) => {
                                    platform_log(LOG_TAG, format!("file seek error: {}", e));
                                }
                            }

                            return Err(FileDownloadError::IO);
                        }

                        let mut f = FileOutput { f };

                        match copy_buf(&mut resp_stream, &mut f).await {
                            Ok(i) => {
                                platform_log(LOG_TAG, format!("bytes copied {}", i));
                                if let Ok(i) = usize::try_from(i) {
                                    if start + i == total {
                                        return Ok(());
                                    }
                                }
                                platform_log(
                                    LOG_TAG,
                                    "inconsistent result of bytes copied and expected total",
                                );
                                return Err(FileDownloadError::IO);
                            }
                            Err(e) => {
                                platform_log(
                                    LOG_TAG,
                                    format!("http stream copy failed with error: {}", e),
                                );
                                return Err(FileDownloadError::IO);
                            }
                        }
                    }
                } else if resp.status_code == 401 {
                    if digest_answer.is_none() {
                        if let Some(www_authenticate_header) =
                            header::search(&resp.headers, b"WWW-Authenticate", false)
                        {
                            if let Some(Ok(answer)) = gba::try_process_401_response(
                                gba_context,
                                host.as_bytes(),
                                conn.cipher_id(),
                                GET,
                                b"\"/\"",
                                None,
                                www_authenticate_header,
                                http_client,
                                security_context,
                            )
                            .await
                            {
                                return download_file(
                                    file_uri,
                                    download_path,
                                    start,
                                    total,
                                    msisdn,
                                    http_client,
                                    gba_context,
                                    security_context,
                                    Some(&answer),
                                )
                                .await;
                            }
                        }
                    }
                } else {
                    return Err(FileDownloadError::Http(
                        resp.status_code,
                        match String::from_utf8(resp.reason_phrase) {
                            Ok(reason_phrase) => reason_phrase,
                            Err(_) => String::from(""),
                        },
                    ));
                }
            }
        }

        Err(FileDownloadError::NetworkIO)
    } else {
        Err(FileDownloadError::MalformedHost)
    }
}

pub fn download_file<'a, 'b: 'a>(
    file_uri: &'b str,
    download_path: &'b str,
    start: usize,
    total: usize,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    digest_answer: Option<&'a DigestAnswerParams>,
) -> BoxFuture<'a, Result<(), FileDownloadError>> {
    async move {
        download_file_inner(
            file_uri,
            download_path,
            start,
            total,
            msisdn,
            http_client,
            gba_context,
            security_context,
            digest_answer,
        )
        .await
    }
    .boxed()
}
