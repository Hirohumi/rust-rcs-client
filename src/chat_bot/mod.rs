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

pub mod chatbot_config;
pub mod chatbot_sip_uri;
pub mod cpim;
pub mod ffi;

use std::sync::Arc;

use futures::{future::BoxFuture, AsyncReadExt, FutureExt};
use rust_rcs_core::{
    ffi::log::platform_log,
    http::{
        request::{Request, GET},
        HttpClient,
    },
    internet::{header, headers::cache_control, Header},
    security::{
        authentication::digest::DigestAnswerParams,
        gba::{self, GbaContext},
        SecurityContext,
    },
};
use url::Url;

const LOG_TAG: &str = "chat_bot";

pub enum RetrieveSpecificChatbotsSuccess {
    Ok(String, Option<String>, u32),
    NotModified(Option<String>, u32),
}

#[derive(Debug)]
pub enum RetrieveSpecificChatbotsError {
    Http(u16, String),
    NetworkIO,
    MalformedUrl,
}

impl RetrieveSpecificChatbotsError {
    pub fn error_code(&self) -> u16 {
        match self {
            RetrieveSpecificChatbotsError::Http(status_code, _) => *status_code,
            RetrieveSpecificChatbotsError::NetworkIO => 0,
            RetrieveSpecificChatbotsError::MalformedUrl => 0,
        }
    }

    pub fn error_string(&self) -> String {
        match self {
            RetrieveSpecificChatbotsError::Http(_, reason_phrase) => String::from(reason_phrase),
            RetrieveSpecificChatbotsError::NetworkIO => String::from("NetworkIO"),
            RetrieveSpecificChatbotsError::MalformedUrl => String::from("MalformedUrl"),
        }
    }
}

async fn retrieve_specific_chatbots_inner(
    specific_chatbots_list_url: &str,
    local_etag: Option<&str>,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    digest_answer: Option<&DigestAnswerParams>,
) -> Result<RetrieveSpecificChatbotsSuccess, RetrieveSpecificChatbotsError> {
    if let Ok(url) = Url::parse(specific_chatbots_list_url) {
        if let Ok(conn) = http_client.connect(&url, false).await {
            let host = url.host_str().unwrap();

            let mut req = Request::new_with_default_headers(GET, host, url.path(), url.query());

            if let Some(msisdn) = msisdn {
                req.headers.push(Header::new(
                    b"X-3GPP-Intended-Identity",
                    format!("tel:{}", msisdn),
                ));
            }

            if let Some(local_etag) = local_etag {
                req.headers
                    .push(Header::new(b"If-None-Match", String::from(local_etag)));
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
                        "retrieve_specific_chatbots_inner resp.status_code = {}",
                        resp.status_code
                    ),
                );

                if (resp.status_code >= 200 && resp.status_code < 300) || resp.status_code == 304 {
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
                }

                if resp.status_code == 200 || resp.status_code == 304 {
                    let response_etag =
                        if let Some(etag_header) = header::search(&resp.headers, b"ETag", false) {
                            match String::from_utf8(etag_header.get_value().to_vec()) {
                                Ok(etag) => Some(etag),
                                Err(_) => None,
                            }
                        } else {
                            None
                        };

                    let expiry = if let Some(cache_control_header) =
                        header::search(&resp.headers, b"Cache-Control", false)
                    {
                        let cc = cache_control::parse(cache_control_header.get_value());
                        cc.max_age
                    } else {
                        0
                    };

                    if resp.status_code == 200 {
                        if let Some(mut resp_stream) = resp_stream {
                            let mut resp_data = Vec::new();
                            match resp_stream.read_to_end(&mut resp_data).await {
                                Ok(size) => {
                                    platform_log(
                                        LOG_TAG,
                                        format!(
                                            "retrieve_specific_chatbots_inner resp.data read {} bytes",
                                            &size
                                        ),
                                    );

                                    if let Ok(resp_string) = String::from_utf8(resp_data) {
                                        platform_log(
                                            LOG_TAG,
                                            format!(
                                                "retrieve_specific_chatbots_inner resp.string = {}",
                                                &resp_string
                                            ),
                                        );

                                        return Ok(RetrieveSpecificChatbotsSuccess::Ok(
                                            resp_string,
                                            response_etag,
                                            expiry,
                                        ));
                                    }
                                }

                                Err(e) => {
                                    platform_log(LOG_TAG, format!("retrieve_specific_chatbots_inner error reading response stream: {:?}", e));
                                }
                            }
                        } else {
                            platform_log(
                                LOG_TAG,
                                "retrieve_specific_chatbots_inner missing response body",
                            );
                        }
                    } else {
                        return Ok(RetrieveSpecificChatbotsSuccess::NotModified(
                            response_etag,
                            expiry,
                        ));
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
                                return retrieve_specific_chatbots(
                                    specific_chatbots_list_url,
                                    local_etag,
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
                    return Err(RetrieveSpecificChatbotsError::Http(
                        resp.status_code,
                        match String::from_utf8(resp.reason_phrase) {
                            Ok(reason_phrase) => reason_phrase,
                            Err(_) => String::from(""),
                        },
                    ));
                }
            }
        }

        Err(RetrieveSpecificChatbotsError::NetworkIO)
    } else {
        Err(RetrieveSpecificChatbotsError::MalformedUrl)
    }
}

pub fn retrieve_specific_chatbots<'a, 'b: 'a>(
    specific_chatbots_list_url: &'b str,
    local_etag: Option<&'b str>,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    digest_answer: Option<&'a DigestAnswerParams>,
) -> BoxFuture<'a, Result<RetrieveSpecificChatbotsSuccess, RetrieveSpecificChatbotsError>> {
    async move {
        retrieve_specific_chatbots_inner(
            specific_chatbots_list_url,
            local_etag,
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

#[derive(Debug)]
pub enum SearchChatbotError {
    Http(u16, String),
    NetworkIO,
    MalformedUrl,
}

impl SearchChatbotError {
    pub fn error_code(&self) -> u16 {
        match self {
            SearchChatbotError::Http(status_code, _) => *status_code,
            SearchChatbotError::NetworkIO => 0,
            SearchChatbotError::MalformedUrl => 0,
        }
    }

    pub fn error_string(&self) -> String {
        match self {
            SearchChatbotError::Http(_, reason_phrase) => String::from(reason_phrase),
            SearchChatbotError::NetworkIO => String::from("NetworkIO"),
            SearchChatbotError::MalformedUrl => String::from("MalformedUrl"),
        }
    }
}

async fn search_chatbot_directory_inner(
    chatbot_directory: &str,
    query: &str,
    start: u32,
    num: u32,
    home_operator: &str,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    digest_answer: Option<&'_ DigestAnswerParams>,
) -> Result<String, SearchChatbotError> {
    let mut first_query = true; // to-do: chatbot_directory might already contain a query
    let mut url_string = String::from(chatbot_directory);
    if query.len() != 0 {
        url_string = format!("{}?q={}", url_string, query);
        first_query = false;
    }

    if start != 0 {
        url_string = if first_query {
            format!("{}?start={}", url_string, query)
        } else {
            format!("{}&start={}", url_string, query)
        };
        first_query = false;
    }

    url_string = if first_query {
        format!(
            "{}&num={}&ho={}&chatbot_version=1_2&client_vendor=Rusty&client_version=1.0.0",
            url_string, num, home_operator
        )
    } else {
        format!(
            "{}&num={}&ho={}&chatbot_version=1_2&client_vendor=Rusty&client_version=1.0.0",
            url_string, num, home_operator
        )
    };

    platform_log(LOG_TAG, format!("url: {}", &url_string));

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
                        "search_chatbot_directory_inner resp.status_code = {}",
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
                        match resp_stream.read_to_end(&mut resp_data).await {
                            Ok(size) => {
                                platform_log(
                                    LOG_TAG,
                                    format!(
                                        "search_chatbot_directory_inner resp.data read {} bytes",
                                        &size
                                    ),
                                );

                                if let Ok(resp_string) = String::from_utf8(resp_data) {
                                    platform_log(
                                        LOG_TAG,
                                        format!(
                                            "search_chatbot_directory_inner resp.string = {}",
                                            &resp_string
                                        ),
                                    );

                                    return Ok(resp_string);
                                }
                            }

                            Err(e) => {
                                platform_log(LOG_TAG, format!("search_chatbot_directory_inner error reading response stream: {:?}", e));
                            }
                        }
                    } else {
                        platform_log(
                            LOG_TAG,
                            "search_chatbot_directory_inner missing response body",
                        );
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
                                return search_chatbot_directory(
                                    chatbot_directory,
                                    query,
                                    start,
                                    num,
                                    home_operator,
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
                    return Err(SearchChatbotError::Http(
                        resp.status_code,
                        match String::from_utf8(resp.reason_phrase) {
                            Ok(reason_phrase) => reason_phrase,
                            Err(_) => String::from(""),
                        },
                    ));
                }
            }
        }

        Err(SearchChatbotError::NetworkIO)
    } else {
        Err(SearchChatbotError::MalformedUrl)
    }
}

pub fn search_chatbot_directory<'a, 'b: 'a>(
    chatbot_directory: &'b str,
    query: &'b str,
    start: u32,
    num: u32,
    home_operator: &'b str,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    digest_answer: Option<&'a DigestAnswerParams>,
) -> BoxFuture<'a, Result<String, SearchChatbotError>> {
    async move {
        search_chatbot_directory_inner(
            chatbot_directory,
            query,
            start,
            num,
            home_operator,
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

pub enum RetrieveChatbotInfoSuccess {
    Ok(String, Option<String>, u32),
    NotModified(Option<String>, u32),
}

#[derive(Debug)]
pub enum RetrieveChatbotInfoError {
    Http(u16, String),
    NetworkIO,
    MalformedUrl,
}

impl RetrieveChatbotInfoError {
    pub fn error_code(&self) -> u16 {
        match self {
            RetrieveChatbotInfoError::Http(status_code, _) => *status_code,
            RetrieveChatbotInfoError::NetworkIO => 0,
            RetrieveChatbotInfoError::MalformedUrl => 0,
        }
    }

    pub fn error_string(&self) -> String {
        match self {
            RetrieveChatbotInfoError::Http(_, reason_phrase) => String::from(reason_phrase),
            RetrieveChatbotInfoError::NetworkIO => String::from("NetworkIO"),
            RetrieveChatbotInfoError::MalformedUrl => String::from("MalformedUrl"),
        }
    }
}

async fn retrieve_chatbot_info_inner(
    bot_domain: &str,
    bot_id: &str,
    local_etag: Option<&str>,
    home_operator: &str,
    home_language: &str,
    msisdn: Option<&str>,
    http_client: &Arc<HttpClient>,
    gba_context: &Arc<GbaContext>,
    security_context: &Arc<SecurityContext>,
    digest_answer: Option<&DigestAnswerParams>,
) -> Result<RetrieveChatbotInfoSuccess, RetrieveChatbotInfoError> {
    let url_string = format!(
        "https://{}/bot?id={}&hl={}&ho={}&v=3",
        bot_domain,
        urlencoding::encode(bot_id),
        home_language,
        home_operator
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

            if let Some(local_etag) = local_etag {
                req.headers
                    .push(Header::new(b"If-None-Match", String::from(local_etag)));
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
                        "retrieve_chatbot_info_inner resp.status_code = {}",
                        resp.status_code
                    ),
                );

                if (resp.status_code >= 200 && resp.status_code < 300) || resp.status_code == 304 {
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
                }

                if resp.status_code == 200 || resp.status_code == 304 {
                    let response_etag =
                        if let Some(etag_header) = header::search(&resp.headers, b"ETag", false) {
                            match String::from_utf8(etag_header.get_value().to_vec()) {
                                Ok(etag) => Some(etag),
                                Err(_) => None,
                            }
                        } else {
                            None
                        };

                    let expiry = if let Some(cache_control_header) =
                        header::search(&resp.headers, b"Cache-Control", false)
                    {
                        let cc = cache_control::parse(cache_control_header.get_value());
                        cc.max_age
                    } else {
                        7 * 24 * 60 * 60
                    };

                    if resp.status_code == 200 {
                        if let Some(mut resp_stream) = resp_stream {
                            let mut resp_data = Vec::new();
                            match resp_stream.read_to_end(&mut resp_data).await {
                                Ok(size) => {
                                    platform_log(
                                        LOG_TAG,
                                        format!(
                                            "retrieve_chatbot_info_inner resp.data read {} bytes",
                                            &size
                                        ),
                                    );

                                    if let Ok(resp_string) = String::from_utf8(resp_data) {
                                        platform_log(
                                            LOG_TAG,
                                            format!(
                                                "retrieve_chatbot_info_inner resp.string = {}",
                                                &resp_string
                                            ),
                                        );

                                        return Ok(RetrieveChatbotInfoSuccess::Ok(
                                            resp_string,
                                            response_etag,
                                            expiry,
                                        ));
                                    }
                                }

                                Err(e) => {
                                    platform_log(LOG_TAG, format!("retrieve_chatbot_info_inner error reading response stream: {:?}", e));
                                }
                            }
                        } else {
                            platform_log(
                                LOG_TAG,
                                "retrieve_chatbot_info_inner missing response body",
                            );
                        }
                    } else {
                        return Ok(RetrieveChatbotInfoSuccess::NotModified(
                            response_etag,
                            expiry,
                        ));
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
                                return retrieve_chatbot_info(
                                    bot_domain,
                                    bot_id,
                                    local_etag,
                                    home_operator,
                                    home_language,
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
                    return Err(RetrieveChatbotInfoError::Http(
                        resp.status_code,
                        match String::from_utf8(resp.reason_phrase) {
                            Ok(reason_phrase) => reason_phrase,
                            Err(_) => String::from(""),
                        },
                    ));
                }
            }
        }

        Err(RetrieveChatbotInfoError::NetworkIO)
    } else {
        Err(RetrieveChatbotInfoError::MalformedUrl)
    }
}

pub fn retrieve_chatbot_info<'a, 'b: 'a>(
    bot_domain: &'b str,
    bot_id: &'b str,
    local_etag: Option<&'b str>,
    home_operator: &'b str,
    home_language: &'b str,
    msisdn: Option<&'b str>,
    http_client: &'b Arc<HttpClient>,
    gba_context: &'b Arc<GbaContext>,
    security_context: &'b Arc<SecurityContext>,
    digest_answer: Option<&'a DigestAnswerParams>,
) -> BoxFuture<'a, Result<RetrieveChatbotInfoSuccess, RetrieveChatbotInfoError>> {
    async move {
        retrieve_chatbot_info_inner(
            bot_domain,
            bot_id,
            local_etag,
            home_operator,
            home_language,
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
