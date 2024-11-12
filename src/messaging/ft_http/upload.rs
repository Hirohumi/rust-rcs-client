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
    fs::File,
    io::{copy, Seek},
    path::MAIN_SEPARATOR,
    sync::Arc,
};

use futures::{future::BoxFuture, AsyncReadExt, FutureExt};
use rust_rcs_core::{
    ffi::log::platform_log,
    http::{
        request::{Request, GET, POST, PUT},
        HttpClient, HttpConnectionHandle,
    },
    internet::{
        body::{
            message_body::MessageBody,
            multipart_body::MultipartBody,
            streamed_body::{StreamSource, StreamedBody},
        },
        header, Body, Header,
    },
    io::Serializable,
    security::{
        authentication::digest::DigestAnswerParams,
        gba::{self, GbaContext},
        SecurityContext,
    },
    util::rand::create_raw_alpha_numeric_string,
};
use url::Url;
use uuid::Uuid;

use super::{
    resume_info_xml::{parse_xml, ResumeInfo},
    FileTransferOverHTTPService,
};

const LOG_TAG: &str = "fthttp";

pub enum FileUploadError {
    Http(u16, String),
    IO,
    MalformedHost,
    NetworkIO,
}

impl FileUploadError {
    pub fn error_code(&self) -> u16 {
        match &self {
            FileUploadError::Http(status_code, _) => *status_code,
            FileUploadError::IO => 0,
            FileUploadError::MalformedHost => 0,
            FileUploadError::NetworkIO => 0,
        }
    }

    pub fn error_string(&self) -> String {
        match &self {
            FileUploadError::Http(_, reason_phrase) => String::from(reason_phrase),
            FileUploadError::IO => String::from("IO"),
            FileUploadError::MalformedHost => String::from("MalformedHost"),
            FileUploadError::NetworkIO => String::from("NetworkIO"),
        }
    }
}

impl Display for FileUploadError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self {
            FileUploadError::Http(status_code, reason_phrase) => {
                f.write_fmt(format_args!("Http {} {}", status_code, reason_phrase))
            }
            FileUploadError::IO => f.write_str("IO"),
            FileUploadError::MalformedHost => f.write_str("MalformedHost"),
            FileUploadError::NetworkIO => f.write_str("NetworkIO"),
        }
    }
}

async fn get_download_info_inner(
    ft_http_cs_uri: &str,
    tid: Uuid,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    digest_answer: Option<&DigestAnswerParams>,
) -> Result<String, FileUploadError> {
    let url_string = format!(
        "{}?tid={}&get_download_info",
        ft_http_cs_uri,
        tid.as_hyphenated().encode_lower(&mut Uuid::encode_buffer())
    );

    if let Ok(url) = Url::parse(&url_string) {
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
                    security_context.preload_auth(gba_context, host, conn.cipher_id(), PUT, None)
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

            if let Ok((resp, resp_stream)) = conn.send(req, |_| {}).await {
                if resp.status_code == 200 {
                    if let Some(mut resp_stream) = resp_stream {
                        let mut resp_data = Vec::new();
                        if let Ok(size) = resp_stream.read_to_end(&mut resp_data).await {
                            platform_log(
                                LOG_TAG,
                                format!("get_download_info_inner resp.data read {} bytes", &size),
                            );
                        }

                        if let Ok(resp_string) = String::from_utf8(resp_data) {
                            platform_log(
                                LOG_TAG,
                                format!("get_download_info_inner resp.string = {}", &resp_string),
                            );

                            return Ok(resp_string);
                        }
                    }
                } else {
                    return Err(FileUploadError::Http(
                        resp.status_code,
                        match String::from_utf8(resp.reason_phrase) {
                            Ok(reason_phrase) => reason_phrase,
                            Err(_) => String::from(""),
                        },
                    ));
                }
            }
        }

        Err(FileUploadError::NetworkIO)
    } else {
        Err(FileUploadError::MalformedHost)
    }
}

fn get_download_info<'a, 'b: 'a>(
    ft_http_cs_uri: &'b str,
    tid: Uuid,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    digest_answer: Option<&'a DigestAnswerParams>,
) -> BoxFuture<'a, Result<String, FileUploadError>> {
    async move {
        get_download_info_inner(
            ft_http_cs_uri,
            tid,
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

async fn resume_upload_put_inner(
    ft_http_cs_uri: &str,
    tid: Uuid,
    file: FileInfo<'_>,
    resume_info: &ResumeInfo,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    digest_answer: Option<&DigestAnswerParams>,
    progress_callback: &Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> Result<String, FileUploadError> {
    if let Ok(url) = Url::parse(&resume_info.data_url) {
        if let Ok(conn) = http_client.connect(&url, false).await {
            let host = url.host_str().unwrap();

            let mut req = Request::new_with_default_headers(PUT, host, url.path(), url.query());

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
                    security_context.preload_auth(gba_context, host, conn.cipher_id(), PUT, None)
                    // fix-me: we do have to perform digest here
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

            let mut file_size = 0;
            if let Ok(mut f) = File::open(file.path) {
                if let Ok(meta) = f.metadata() {
                    file_size = meta.len();
                } else {
                    if let Ok(size) = f.seek(std::io::SeekFrom::End(0)) {
                        file_size = size;
                    }
                }
            }

            let file_size = match usize::try_from(file_size) {
                Ok(file_size) => {
                    if file_size > 0 {
                        file_size
                    } else {
                        return Err(FileUploadError::IO);
                    }
                }
                Err(_) => return Err(FileUploadError::IO),
            };

            if resume_info.range_end + 1 >= file_size {
                return Err(FileUploadError::IO);
            }

            let http_body = Body::Streamed(StreamedBody {
                stream_size: file_size - resume_info.range_end,
                stream_source: StreamSource::File((String::from(file.path), resume_info.range_end)),
            });

            req.headers
                .push(Header::new("Content-Type", "application/octet-stream"));

            req.headers.push(Header::new(
                "Content-Range",
                format!("{}-{}/{}", resume_info.range_end, file_size - 1, file_size),
            ));

            let progress_total = http_body.estimated_size();

            req.headers
                .push(Header::new("Content-Length", format!("{}", progress_total)));

            req.body = Some(http_body);

            let progress_callback_ = Arc::clone(progress_callback);

            if let Ok((resp, _)) = conn
                .send(req, move |written| {
                    if let Ok(current) = u32::try_from(written) {
                        if let Ok(total) = i32::try_from(progress_total) {
                            progress_callback_(current, total);
                        } else {
                            progress_callback_(current, -1);
                        }
                    }
                })
                .await
            {
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
                                    false, // fix-me: we do have http body here
                                );
                            }
                        }
                    }

                    return get_download_info(
                        ft_http_cs_uri,
                        tid,
                        msisdn,
                        http_client,
                        gba_context,
                        security_context,
                        None,
                    )
                    .await;
                } else if resp.status_code == 401 {
                    if digest_answer.is_none() {
                        if let Some(www_authenticate_header) =
                            header::search(&resp.headers, b"WWW-Authenticate", false)
                        {
                            if let Some(Ok(answer)) = gba::try_process_401_response(
                                gba_context,
                                host.as_bytes(),
                                conn.cipher_id(),
                                PUT,
                                b"\"/\"",
                                None,
                                www_authenticate_header,
                                http_client,
                                security_context,
                            )
                            .await
                            {
                                return resume_upload_put(
                                    ft_http_cs_uri,
                                    tid,
                                    file,
                                    resume_info,
                                    msisdn,
                                    http_client,
                                    gba_context,
                                    security_context,
                                    Some(&answer),
                                    progress_callback,
                                )
                                .await;
                            }
                        }
                    }
                } else {
                    return Err(FileUploadError::Http(
                        resp.status_code,
                        match String::from_utf8(resp.reason_phrase) {
                            Ok(reason_phrase) => reason_phrase,
                            Err(_) => String::from(""),
                        },
                    ));
                }
            }
        }

        Err(FileUploadError::NetworkIO)
    } else {
        Err(FileUploadError::MalformedHost)
    }
}

fn resume_upload_put<'a, 'b: 'a>(
    ft_http_cs_uri: &'b str,
    tid: Uuid,
    file_info: FileInfo<'b>,
    resume_info: &'b ResumeInfo,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    digest_answer: Option<&'a DigestAnswerParams>,
    progress_callback: &'b Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> BoxFuture<'a, Result<String, FileUploadError>> {
    async move {
        resume_upload_put_inner(
            ft_http_cs_uri,
            tid,
            file_info,
            resume_info,
            msisdn,
            http_client,
            gba_context,
            security_context,
            digest_answer,
            progress_callback,
        )
        .await
    }
    .boxed()
}

async fn resume_upload_check_inner(
    ft_http_cs_uri: &str,
    tid: Uuid,
    file: FileInfo<'_>,
    thumbnail: Option<FileInfo<'_>>,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    digest_answer: Option<&DigestAnswerParams>,
    progress_callback: &Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> Result<String, FileUploadError> {
    let url_string = format!(
        "{}?tid={}&get_upload_info",
        ft_http_cs_uri,
        tid.as_hyphenated().encode_lower(&mut Uuid::encode_buffer())
    );

    if let Ok(url) = Url::parse(&url_string) {
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

            if let Ok((resp, resp_stream)) = conn.send(req, |_| {}).await {
                platform_log(
                    LOG_TAG,
                    format!(
                        "resume_upload_check_inner resp.status_code = {}",
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
                        let mut resp_data = Vec::new();

                        if let Ok(size) = resp_stream.read_to_end(&mut resp_data).await {
                            platform_log(
                                LOG_TAG,
                                format!("resume_upload_check_inner resp.data read {} bytes", &size),
                            );

                            if let Some(resume_info) = parse_xml(&resp_data) {
                                return resume_upload_put(
                                    ft_http_cs_uri,
                                    tid,
                                    file,
                                    &resume_info,
                                    msisdn,
                                    http_client,
                                    gba_context,
                                    security_context,
                                    None,
                                    progress_callback,
                                )
                                .await;
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
                                return resume_upload_check(
                                    ft_http_cs_uri,
                                    tid,
                                    file,
                                    thumbnail,
                                    msisdn,
                                    http_client,
                                    gba_context,
                                    security_context,
                                    Some(&answer),
                                    progress_callback,
                                )
                                .await;
                            }
                        }
                    }
                } else if resp.status_code == 404 {
                    return upload_file_inner(
                        ft_http_cs_uri,
                        tid,
                        file,
                        thumbnail,
                        msisdn,
                        http_client,
                        gba_context,
                        security_context,
                        progress_callback,
                    )
                    .await;
                } else {
                    return Err(FileUploadError::Http(
                        resp.status_code,
                        match String::from_utf8(resp.reason_phrase) {
                            Ok(reason_phrase) => reason_phrase,
                            Err(_) => String::from(""),
                        },
                    ));
                }
            }
        }

        Err(FileUploadError::NetworkIO)
    } else {
        Err(FileUploadError::MalformedHost)
    }
}

fn resume_upload_check<'a, 'b: 'a>(
    ft_http_cs_uri: &'b str,
    tid: Uuid,
    file: FileInfo<'b>,
    thumbnail: Option<FileInfo<'b>>,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    digest_answer: Option<&'a DigestAnswerParams>,
    progress_callback: &'b Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> BoxFuture<'a, Result<String, FileUploadError>> {
    async move {
        resume_upload_check_inner(
            ft_http_cs_uri,
            tid,
            file,
            thumbnail,
            msisdn,
            http_client,
            gba_context,
            security_context,
            digest_answer,
            progress_callback,
        )
        .await
    }
    .boxed()
}

async fn resume_upload(
    ft_http_cs_uri: &str,
    tid: Uuid,
    file: FileInfo<'_>,
    thumbnail: Option<FileInfo<'_>>,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    progress_callback: &Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> Result<String, FileUploadError> {
    resume_upload_check(
        ft_http_cs_uri,
        tid,
        file,
        thumbnail,
        msisdn,
        http_client,
        gba_context,
        security_context,
        None,
        progress_callback,
    )
    .await
}

async fn upload_file_actual_content_post_inner(
    url: &Url,
    conn: &HttpConnectionHandle,
    tid: Uuid,
    file: FileInfo<'_>,
    thumbnail: Option<FileInfo<'_>>,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    digest_answer: Option<&DigestAnswerParams>,
    progress_callback: &Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> Result<String, FileUploadError> {
    let host = url.host_str().unwrap();

    let mut req = Request::new_with_default_headers(POST, host, url.path(), url.query());

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
            security_context.preload_auth(gba_context, host, conn.cipher_id(), POST, None)
            // fix-me: we do have to perform digest here
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

    let boundary = create_raw_alpha_numeric_string(16);
    let boundary_ = String::from_utf8_lossy(&boundary);

    let tid_string = String::from(tid.as_hyphenated().encode_lower(&mut Uuid::encode_buffer()));

    let tid_payload = Body::Message(MessageBody {
        headers: [
            Header::new("Content-Disposition", "form-data; name=\"tid\""),
            Header::new("Content-Type", "text/plain; charset=utf-8"),
            Header::new("Content-Length", format!("{}", tid_string.len())),
        ]
        .to_vec(),
        body: Arc::new(Body::Raw(tid_string.into_bytes())),
    });

    let thumbnail_payload = match &thumbnail {
        Some(thumbnail) => {
            let mut file_size = 0;
            if let Ok(mut f) = File::open(thumbnail.path) {
                if let Ok(meta) = f.metadata() {
                    file_size = meta.len();
                } else {
                    if let Ok(size) = f.seek(std::io::SeekFrom::End(0)) {
                        file_size = size;
                    }
                }
            }

            let stream_size = match usize::try_from(file_size) {
                Ok(file_size) => {
                    if file_size > 0 {
                        file_size
                    } else {
                        return Err(FileUploadError::IO);
                    }
                }
                Err(_) => return Err(FileUploadError::IO),
            };

            Some(Body::Message(MessageBody {
                headers: [
                    Header::new(
                        "Content-Disposition",
                        format!(
                            "form-data; name=\"Thumbnail\"; filename=\"{}\"",
                            thumbnail.name
                        ),
                    ),
                    Header::new("Content-Type", String::from(thumbnail.mime)),
                    Header::new("Content-Transfer-Encoding", "binary"),
                    Header::new(
                        "Content-Range",
                        format!("0-{}/{}", stream_size - 1, stream_size),
                    ),
                    Header::new("Content-Length", format!("{}", stream_size)),
                ]
                .to_vec(),
                body: Arc::new(Body::Streamed(StreamedBody {
                    stream_size,
                    stream_source: StreamSource::File((String::from(thumbnail.path), 0)),
                })),
            }))
        }
        None => None,
    };

    let mut file_size = 0;
    if let Ok(mut f) = File::open(file.path) {
        if let Ok(meta) = f.metadata() {
            file_size = meta.len();
        } else {
            if let Ok(size) = f.seek(std::io::SeekFrom::End(0)) {
                file_size = size;
            }
        }
    }

    let stream_size = match usize::try_from(file_size) {
        Ok(file_size) => {
            if file_size > 0 {
                file_size
            } else {
                return Err(FileUploadError::IO);
            }
        }
        Err(_) => return Err(FileUploadError::IO),
    };

    let file_payload = Body::Message(MessageBody {
        headers: [
            Header::new(
                "Content-Disposition",
                format!("form-data; name=\"File\"; filename=\"{}\"", file.name),
            ),
            Header::new("Content-Type", String::from(file.mime)),
            Header::new("Content-Transfer-Encoding", "binary"),
            Header::new(
                "Content-Range",
                format!("0-{}/{}", stream_size - 1, stream_size),
            ),
            Header::new("Content-Length", format!("{}", stream_size)),
        ]
        .to_vec(),
        body: Arc::new(Body::Streamed(StreamedBody {
            stream_size,
            stream_source: StreamSource::File((String::from(file.path), 0)),
        })),
    });

    req.headers.push(Header::new(
        "Content-Type",
        format!("multipart/form-data; boundary={}", boundary_),
    ));

    let http_body = Body::Multipart(MultipartBody {
        boundary,
        parts: match thumbnail_payload {
            Some(thumbnail_payload) => [
                Arc::new(tid_payload),
                Arc::new(thumbnail_payload),
                Arc::new(file_payload),
            ]
            .to_vec(),
            None => [Arc::new(tid_payload), Arc::new(file_payload)].to_vec(),
        },
    });

    let progress_total = http_body.estimated_size();

    req.headers
        .push(Header::new("Content-Length", format!("{}", progress_total)));

    req.body = Some(http_body);

    let progress_callback_ = Arc::clone(progress_callback);

    if let Ok((resp, resp_stream)) = conn
        .send(req, move |written| {
            if let Ok(current) = u32::try_from(written) {
                if let Ok(total) = i32::try_from(progress_total) {
                    progress_callback_(current, total);
                } else {
                    progress_callback_(current, -1);
                }
            }
        })
        .await
    {
        platform_log(
            LOG_TAG,
            format!(
                "upload_file_actual_content_post_inner resp.status_code = {}",
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
                            false, // fix-me: we do have http body here
                        );
                    }
                }
            }

            if let Some(mut resp_stream) = resp_stream {
                let mut resp_data = Vec::new();
                if let Ok(size) = resp_stream.read_to_end(&mut resp_data).await {
                    platform_log(
                        LOG_TAG,
                        format!(
                            "upload_file_actual_content_post_inner resp.data read {} bytes",
                            &size
                        ),
                    );

                    if let Ok(resp_string) = String::from_utf8(resp_data) {
                        platform_log(
                            LOG_TAG,
                            format!(
                                "upload_file_actual_content_post_inner resp.string = {}",
                                &resp_string
                            ),
                        );

                        return Ok(resp_string);
                    }
                }
            }
        } else {
            return Err(FileUploadError::Http(
                resp.status_code,
                match String::from_utf8(resp.reason_phrase) {
                    Ok(reason_phrase) => reason_phrase,
                    Err(_) => String::from(""),
                },
            ));
        }
    }

    Err(FileUploadError::MalformedHost)
}

fn upload_file_actual_content_post<'a, 'b: 'a>(
    url: &'a Url,
    conn: &'a HttpConnectionHandle,
    tid: Uuid,
    file: FileInfo<'b>,
    thumbnail: Option<FileInfo<'b>>,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    digest_answer: Option<&'a DigestAnswerParams>,
    progress_callback: &'b Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> BoxFuture<'a, Result<String, FileUploadError>> {
    async move {
        upload_file_actual_content_post_inner(
            url,
            conn,
            tid,
            file,
            thumbnail,
            msisdn,
            http_client,
            gba_context,
            security_context,
            digest_answer,
            progress_callback,
        )
        .await
    }
    .boxed()
}

async fn upload_file_initial_empty_post_inner(
    url: &Url,
    conn: &HttpConnectionHandle,
    tid: Uuid,
    file: FileInfo<'_>,
    thumbnail: Option<FileInfo<'_>>,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    progress_callback: &Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> Result<String, FileUploadError> {
    let host = url.host_str().unwrap();

    let mut req = Request::new_with_default_headers(POST, host, url.path(), url.query());

    if let Some(msisdn) = msisdn {
        req.headers.push(Header::new(
            b"X-3GPP-Intended-Identity",
            format!("tel:{}", msisdn),
        ));
    }

    if let Ok((resp, _)) = conn.send(req, |_| {}).await {
        platform_log(
            LOG_TAG,
            format!(
                "upload_file_initial_empty_post_inner resp.status_code = {}",
                resp.status_code
            ),
        );

        if resp.status_code == 204 {
            if let Ok(conn) = http_client.connect(url, false).await {
                return upload_file_actual_content_post(
                    url,
                    &conn,
                    tid,
                    file,
                    thumbnail,
                    msisdn,
                    http_client,
                    gba_context,
                    security_context,
                    None,
                    progress_callback,
                )
                .await;
            } else {
                return Err(FileUploadError::NetworkIO);
            }
        } else if resp.status_code == 401 {
            if let Some(www_authenticate_header) =
                header::search(&resp.headers, b"WWW-Authenticate", false)
            {
                if let Ok(conn) = http_client.connect(url, false).await {
                    // to-do: could also be a simple Digest with ftHTTPCSUser and ftHTTPCSPwd
                    if let Some(Ok(authorization)) = gba::try_process_401_response(
                        gba_context,
                        host.as_bytes(),
                        conn.cipher_id(),
                        POST,
                        b"\"/\"",
                        None,
                        www_authenticate_header,
                        http_client,
                        security_context,
                    )
                    .await
                    {
                        return upload_file_actual_content_post(
                            &url,
                            &conn,
                            tid,
                            file,
                            thumbnail,
                            msisdn,
                            http_client,
                            gba_context,
                            security_context,
                            Some(&authorization),
                            progress_callback,
                        )
                        .await;
                    }
                } else {
                    return Err(FileUploadError::NetworkIO);
                }
            }
        } else {
            return Err(FileUploadError::Http(
                resp.status_code,
                match String::from_utf8(resp.reason_phrase) {
                    Ok(reason_phrase) => reason_phrase,
                    Err(_) => String::from(""),
                },
            ));
        }
    }

    Err(FileUploadError::NetworkIO)
}

fn upload_file_initial_empty_post<'a, 'b: 'a>(
    url: &'a Url,
    conn: &'a HttpConnectionHandle,
    tid: Uuid,
    file: FileInfo<'b>,
    thumbnail: Option<FileInfo<'b>>,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    progress_callback: &'b Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> BoxFuture<'a, Result<String, FileUploadError>> {
    async move {
        upload_file_initial_empty_post_inner(
            url,
            conn,
            tid,
            file,
            thumbnail,
            msisdn,
            http_client,
            gba_context,
            security_context,
            progress_callback,
        )
        .await
    }
    .boxed()
}

async fn upload_file_inner(
    ft_http_cs_uri: &str,
    tid: Uuid,
    file: FileInfo<'_>,
    thumbnail: Option<FileInfo<'_>>,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    progress_callback: &Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> Result<String, FileUploadError> {
    platform_log(LOG_TAG, "calling upload_file_inner()");

    if let Ok(url) = Url::parse(ft_http_cs_uri) {
        if let Ok(conn) = http_client.connect(&url, false).await {
            return upload_file_initial_empty_post(
                &url,
                &conn,
                tid,
                file,
                thumbnail,
                msisdn,
                http_client,
                gba_context,
                security_context,
                progress_callback,
            )
            .await;
        }

        Err(FileUploadError::NetworkIO)
    } else {
        Err(FileUploadError::MalformedHost)
    }
}

pub struct FileInfo<'a> {
    pub path: &'a str,
    pub name: &'a str,
    pub mime: &'a str,
    pub hash: Option<&'a str>,
}

pub fn upload_file<'a, 'b: 'a>(
    ft_http_service: &'b Arc<FileTransferOverHTTPService>,
    ft_http_cs_uri: &'b str,
    tid: Uuid,
    file: FileInfo<'b>,
    thumbnail: Option<FileInfo<'b>>,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    progress_callback: &'b Arc<Box<dyn Fn(u32, i32) + Send + Sync>>,
) -> BoxFuture<'a, Result<String, FileUploadError>> {
    let mut is_known_task = false;

    {
        let mut guard = ft_http_service.recorded_tids.lock().unwrap();
        for recorded_tid in &*guard {
            if &tid == recorded_tid {
                is_known_task = true;
                break;
            }
        }

        if !is_known_task {
            guard.push(tid);
        }
    }

    if is_known_task {
        async move {
            resume_upload(
                ft_http_cs_uri,
                tid,
                file,
                thumbnail,
                msisdn,
                http_client,
                gba_context,
                security_context,
                progress_callback,
            )
            .await
        }
        .boxed()
    } else {
        async move {
            upload_file_inner(
                ft_http_cs_uri,
                tid,
                file,
                thumbnail,
                msisdn,
                http_client,
                gba_context,
                security_context,
                progress_callback,
            )
            .await
        }
        .boxed()
    }
}
