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

extern crate chrono;
extern crate cookie;
extern crate futures;
extern crate url;

use std::fmt;
use std::str;
use std::sync::Arc;

use chrono::{DateTime, Duration, FixedOffset, Utc};

use cookie::{Cookie, CookieJar};

use futures::future::{BoxFuture, FutureExt};
use futures::io::AsyncReadExt;

use rust_rcs_core::security::authentication::digest::DigestAnswerParams;
use tokio::time::timeout;
use url::Url;

use rust_rcs_core::ffi::log::platform_log;

use rust_rcs_core::http::request::{Request, GET};

use rust_rcs_core::internet::header::{self, Header, HeaderSearch};

use rust_rcs_core::security::gba::{self, GbaContext};

use rust_rcs_core::third_gen_pp::{CountryCode, NetworkCode, ThreeDigit};

use rust_rcs_core::util::raw_string::ToInt;

use crate::context::Context;

use super::characteristic::Characteristic;
use super::ims_application::GetImsApplication;
use super::local_provisioning_doc::LocalProvisioningDoc;
use super::local_provisioning_doc::{
    load_provisionging_doc, store_provisionging_doc, ProvisiongingInfo,
};
use super::rcs_application::GetRcsApplication;
use super::wap_provisioning_doc::DEFAULT_APPLICATION_PORT;
use super::wap_provisioning_doc::{AccessControlInfo, WapProvisioningDoc};

const LOG_TAG: &str = "dm";

const ShouldReceiveZeroPortSMS: i32 = 1;

pub enum RetryReason {
    RequireCellularNetworkOrMsisdn,
    // RequireOtp(u16, String, String), // sms port, https host, cookie b
    ServiceUnavailable,
    IO,
    NetworkIO,
}

impl fmt::Debug for RetryReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RetryReason::RequireCellularNetworkOrMsisdn => {
                write!(f, "RequireCellularNetworkOrMsisdn")
            }

            // RetryReason::RequireOtp(port, https_host, cookie_b) => {
            //     write!(f, "RequireOtp on port {}, pending https session with host {}, cookie {}", port, https_host, cookie_b)
            // }
            RetryReason::ServiceUnavailable => {
                write!(f, "ServiceUnavailable")
            }

            RetryReason::IO => {
                write!(f, "IO")
            }

            RetryReason::NetworkIO => {
                write!(f, "NetworkIO")
            }
        }
    }
}

pub enum DeviceConfigurationStatus {
    InvalidArgument,
    Forbidden,
    Retry(RetryReason, u32),
}

impl fmt::Debug for DeviceConfigurationStatus {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            DeviceConfigurationStatus::InvalidArgument => {
                write!(f, "InvalidArgument")
            }

            DeviceConfigurationStatus::Forbidden => {
                write!(f, "Forbidden")
            }

            DeviceConfigurationStatus::Retry(reason, timeout) => {
                write!(f, "Retry after {} because of reason: {:?}", timeout, reason)
            }
        }
    }
}

enum ConfigurationServer {
    Default(CountryCode, NetworkCode),
    PreConfiguredDefault(String, String, String),
    Additional(String, bool),
}

impl ConfigurationServer {
    fn identifier(&self) -> String {
        match self {
            ConfigurationServer::Default(mcc, mnc) => {
                format!(
                    "config.rcs.mnc{}.mcc{}.pub.3gppnetwork.org",
                    mnc.string_repr(),
                    mcc.string_repr()
                )
            }
            ConfigurationServer::PreConfiguredDefault(identifier, _, _) => identifier.clone(),
            ConfigurationServer::Additional(fqdn, _) => fqdn.clone(),
        }
    }

    fn http_uri(&self) -> String {
        match self {
            ConfigurationServer::Default(mcc, mnc) => {
                format!(
                    "http://config.rcs.mnc{}.mcc{}.pub.3gppnetwork.org",
                    mnc.string_repr(),
                    mcc.string_repr()
                )
            }
            ConfigurationServer::PreConfiguredDefault(_, http_uri, _) => http_uri.clone(),
            ConfigurationServer::Additional(fqdn, _) => {
                format!("http://{}", fqdn)
            }
        }
    }

    fn https_uri(&self) -> String {
        match self {
            ConfigurationServer::Default(mcc, mnc) => {
                format!(
                    "https://config.rcs.mnc{}.mcc{}.pub.3gppnetwork.org",
                    mnc.string_repr(),
                    mcc.string_repr()
                )
            }
            ConfigurationServer::PreConfiguredDefault(_, _, https_uri) => https_uri.clone(),
            ConfigurationServer::Additional(fqdn, _) => {
                format!("https://{}", fqdn)
            }
        }
    }
}

fn try_otp_configuration_request<'a, 'b: 'a>(
    server: ConfigurationServer,
    access_control_info: Option<AccessControlInfo<'a>>,
    imei: &'b str,
    imsi: &'b str,
    msisdn: Option<&'b str>,
    vers_version: i64,
    token_string: Option<&'b str>,
    otp: &'b str,
    cookie_jar: Option<&'b CookieJar>,
    is_over_cellular: bool,
    context: Arc<Context>,
    gba_context: &'a GbaContext,
    progress_callback: Arc<Box<dyn Fn(i32) + Send + Sync + 'b>>,
) -> BoxFuture<'a, Result<Option<String>, DeviceConfigurationStatus>> {
    async move {
        try_otp_configuration_request_inner(
            server,
            access_control_info,
            imei,
            imsi,
            msisdn,
            vers_version,
            token_string,
            otp,
            cookie_jar,
            is_over_cellular,
            context,
            gba_context,
            progress_callback,
        )
        .await
    }
    .boxed()
}

async fn try_otp_configuration_request_inner(
    server: ConfigurationServer,
    access_control_info: Option<AccessControlInfo<'_>>,
    imei: &str,
    imsi: &str,
    msisdn: Option<&str>,
    vers_version: i64,
    token_string: Option<&str>,
    otp: &str,
    cookie_jar: Option<&CookieJar>,
    is_over_cellular: bool,
    context: Arc<Context>,
    gba_context: &GbaContext,
    progress_callback: Arc<Box<dyn Fn(i32) + Send + Sync + '_>>,
) -> Result<Option<String>, DeviceConfigurationStatus> {
    let https_uri = server.https_uri();

    let uri = format!("{}?otp={}", https_uri, otp);

    if let Ok(url) = Url::parse(&uri) {
        match context.get_http_client().connect(&url, false).await {
            Ok(connection) => {
                let host = url.host_str().unwrap();

                let mut req = Request::new_with_default_headers(GET, host, url.path(), url.query());

                if let Some(msisdn) = msisdn {
                    req.headers.push(Header::new(
                        b"X-3GPP-Intended-Identity",
                        format!("tel:{}", msisdn),
                    ));
                }

                if let Some(cookie_jar) = cookie_jar {
                    for cookie in cookie_jar.iter() {
                        req.headers.push(Header::new(b"Cookie", cookie.to_string()));
                    }
                }

                match connection.send(req, |_| {}).await {
                    Ok((resp, resp_stream)) => {
                        platform_log(
                            LOG_TAG,
                            format!(
                                "try_otp_configuration_request_inner resp.status_code = {}",
                                resp.status_code
                            ),
                        );

                        if resp.status_code == 200 {
                            if let Some(mut resp_stream) = resp_stream {
                                let mut resp_data = Vec::new();
                                if let Ok(size) = resp_stream.read_to_end(&mut resp_data).await {
                                    platform_log(
                                        LOG_TAG,
                                        format!(
                                            "try_otp_configuration_request_inner resp.data read {} bytes",
                                            &size
                                        ),
                                    );
                                    if let Ok(resp_string) = String::from_utf8(resp_data) {
                                        platform_log(
                                            LOG_TAG,
                                            format!(
                                                "try_otp_configuration_request_inner resp.string = {}",
                                                &resp_string
                                            ),
                                        );

                                        match resp_string.parse::<WapProvisioningDoc>() {
                                            Ok(wap_provisioning_doc) => {
                                                return handle_config_for_server(
                                                    server,
                                                    access_control_info,
                                                    imei,
                                                    imsi,
                                                    msisdn,
                                                    &wap_provisioning_doc,
                                                    context,
                                                    gba_context,
                                                    progress_callback,
                                                )
                                                .await;
                                            }

                                            Err(e) => {
                                                platform_log(
                                                    LOG_TAG,
                                                    format!("error parsing resp: {}", e),
                                                );
                                            }
                                        }
                                    }
                                }
                            }
                        } else if resp.status_code == 404 {
                        } else if resp.status_code == 408 {
                        } else if resp.status_code == 511 {
                        }
                    }

                    Err(e) => {
                        platform_log(
                            LOG_TAG,
                            format!(
                                "try_otp_configuration_request_inner send failed with error {:?}",
                                e
                            ),
                        );
                    }
                }
            }

            Err(e) => {
                platform_log(
                    LOG_TAG,
                    format!(
                        "try_otp_configuration_request_inner connect failed with error {:?}",
                        e
                    ),
                );
            }
        }
    }

    Err(DeviceConfigurationStatus::Retry(RetryReason::NetworkIO, 0))
}

fn try_https_configuration_request<'a, 'b: 'a>(
    server: ConfigurationServer,
    access_control_info: Option<AccessControlInfo<'a>>,
    imei: &'b str,
    imsi: &'b str,
    msisdn: Option<&'b str>,
    vers_version: i64,
    token_string: Option<&'b str>,
    cookie_jar: Option<&'b CookieJar>,
    digest_answer: Option<&'a DigestAnswerParams>,
    is_over_cellular: bool,
    context: Arc<Context>,
    gba_context: &'b GbaContext,
    progress_callback: Arc<Box<dyn Fn(i32) + Send + Sync + 'b>>,
) -> BoxFuture<'a, Result<Option<String>, DeviceConfigurationStatus>> {
    async move {
        try_https_configuration_request_inner(
            server,
            access_control_info,
            imei,
            imsi,
            msisdn,
            vers_version,
            token_string,
            cookie_jar,
            digest_answer,
            is_over_cellular,
            context,
            gba_context,
            progress_callback,
        )
        .await
    }
    .boxed()
}

async fn try_https_configuration_request_inner(
    server: ConfigurationServer,
    access_control_info: Option<AccessControlInfo<'_>>,
    imei: &str,
    imsi: &str,
    msisdn: Option<&str>,
    vers_version: i64,
    token_string: Option<&str>,
    cookie_jar: Option<&CookieJar>,
    digest_answer: Option<&DigestAnswerParams>,
    is_over_cellular: bool,
    context: Arc<Context>,
    gba_context: &GbaContext,
    progress_callback: Arc<Box<dyn Fn(i32) + Send + Sync + '_>>,
) -> Result<Option<String>, DeviceConfigurationStatus> {
    let https_uri = server.https_uri();

    let uri = match (msisdn, token_string) {
        (Some(msisdn), Some(token_string)) => format!("{}?vers={}&IMSI={}&IMEI={}&msisdn={}&token={}&Channel_ID=&rcs_version=11.0&rcs_profile=UP_2.4&provisioning_version=5.0&app=ap2001&app=ap2002&app=urn%3Aoma%3Amo%3Aext-3gpp-ims%3A1.0", https_uri, vers_version, imsi, imei, msisdn, token_string),
        (None, Some(token_string)) => format!("{}?vers={}&IMSI={}&IMEI={}&msisdn=&token={}&Channel_ID=&rcs_version=11.0&rcs_profile=UP_2.4&provisioning_version=5.0&app=ap2001&app=ap2002&app=urn%3Aoma%3Amo%3Aext-3gpp-ims%3A1.0", https_uri, vers_version, imsi, imei, token_string),
        (Some(msisdn), None) => format!("{}?vers={}&IMSI={}&IMEI={}&msisdn={}&token=&SMS_port={}&Channel_ID=&rcs_version=11.0&rcs_profile=UP_2.4&provisioning_version=5.0&app=ap2001&app=ap2002&app=urn%3Aoma%3Amo%3Aext-3gpp-ims%3A1.0", https_uri, vers_version, imsi, imei, msisdn, DEFAULT_APPLICATION_PORT),
        (None, None) => format!("{}?vers={}&IMSI={}&IMEI={}&msisdn=&token=&SMS_port={}&Channel_ID=&rcs_version=11.0&rcs_profile=UP_2.4&provisioning_version=5.0&app=ap2001&app=ap2002&app=urn%3Aoma%3Amo%3Aext-3gpp-ims%3A1.0", https_uri, vers_version, imsi, imei, DEFAULT_APPLICATION_PORT),
    };

    if let Ok(url) = Url::parse(&uri) {
        match context.get_http_client().connect(&url, false).await {
            Ok(connection) => {
                let host = url.host_str().unwrap();

                let mut req = Request::new_with_default_headers(GET, host, url.path(), url.query());

                if let Some(msisdn) = msisdn {
                    req.headers.push(Header::new(
                        b"X-3GPP-Intended-Identity",
                        format!("tel:{}", msisdn),
                    ));
                }

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

                if let Some(cookie_jar) = cookie_jar {
                    for cookie in cookie_jar.iter() {
                        req.headers.push(Header::new(b"Cookie", cookie.to_string()));
                    }
                }

                match connection.send(req, |_| {}).await {
                    Ok((resp, resp_stream)) => {
                        platform_log(
                            LOG_TAG,
                            format!(
                                "try_https_configuration_request_inner resp.status_code = {}",
                                resp.status_code
                            ),
                        );

                        if resp.status_code == 200 {
                            if let Some(mut resp_stream) = resp_stream {
                                let mut resp_data = Vec::new();
                                if let Ok(size) = resp_stream.read_to_end(&mut resp_data).await {
                                    platform_log(LOG_TAG, format!("try_https_configuration_request_inner resp.data read {}", &size));
                                    if let Ok(resp_string) = String::from_utf8(resp_data) {
                                        platform_log(
                                            LOG_TAG,
                                            format!(
                                            "try_https_configuration_request_inner resp.string = {}",
                                            &resp_string
                                        ),
                                        );

                                        match resp_string.parse::<WapProvisioningDoc>() {
                                            Ok(wap_provisioning_doc) => {
                                                if let Some(port) =
                                                    wap_provisioning_doc.get_sms_policy()
                                                {
                                                    if port == 0 {
                                                        progress_callback(ShouldReceiveZeroPortSMS);
                                                    }

                                                    let mut cookie_jar = CookieJar::new();
                                                    for header in HeaderSearch::new(
                                                        &resp.headers,
                                                        b"Set-Cookie",
                                                        false,
                                                    ) {
                                                        if let Ok(cookie_string) =
                                                            str::from_utf8(header.get_value())
                                                        {
                                                            if let Ok(cookie) =
                                                                cookie_string.parse::<Cookie>()
                                                            {
                                                                cookie_jar.add(cookie);
                                                            }
                                                        }
                                                    }

                                                    let mut otp_receiver = context.subscribe_otp();

                                                    match timeout(
                                                        std::time::Duration::from_secs(64),
                                                        otp_receiver.recv(),
                                                    )
                                                    .await
                                                    {
                                                        Ok(r) => match r {
                                                            Ok(otp) => {
                                                                return try_otp_configuration_request(server, access_control_info, imei, imsi, msisdn, vers_version, token_string, &otp, Some(&cookie_jar), is_over_cellular, context, gba_context, progress_callback).await;
                                                            }
                                                            Err(e) => {
                                                                platform_log(
                                                                        LOG_TAG,
                                                                        format!("failed to receive otp with error {}", e),
                                                                    );
                                                            }
                                                        },
                                                        Err(e) => {
                                                            platform_log(
                                                                LOG_TAG,
                                                                format!(
                                                                    "failed to receive otp in {}",
                                                                    e
                                                                ),
                                                            );
                                                        }
                                                    }
                                                } else {
                                                    return handle_config_for_server(
                                                        server,
                                                        access_control_info,
                                                        imei,
                                                        imsi,
                                                        msisdn,
                                                        &wap_provisioning_doc,
                                                        context,
                                                        gba_context,
                                                        progress_callback,
                                                    )
                                                    .await;
                                                }
                                            }

                                            Err(e) => {
                                                platform_log(
                                                    LOG_TAG,
                                                    format!("error parsing resp: {}", e),
                                                );
                                            }
                                        }
                                    }
                                }
                            } else {
                                platform_log(LOG_TAG, "try_https_configuration_request_inner resp has no readable stream")
                            }

                            let mut has_cookie_b = false;

                            let mut cookie_jar = CookieJar::new();
                            for header in HeaderSearch::new(&resp.headers, b"Set-Cookie", false) {
                                if let Ok(cookie_string) = str::from_utf8(header.get_value()) {
                                    if let Ok(cookie) = cookie_string.parse::<Cookie>() {
                                        cookie_jar.add(cookie);
                                        has_cookie_b = true;
                                    }
                                }
                            }

                            if has_cookie_b {
                                platform_log(LOG_TAG, "try_https_configuration_request_inner subscribe for otp before continue");

                                let mut otp_receiver = context.subscribe_otp();

                                match timeout(
                                    std::time::Duration::from_secs(64),
                                    otp_receiver.recv(),
                                )
                                .await
                                {
                                    Ok(r) => match r {
                                        Ok(otp) => {
                                            return try_otp_configuration_request(
                                                server,
                                                access_control_info,
                                                imei,
                                                imsi,
                                                msisdn,
                                                vers_version,
                                                token_string,
                                                &otp,
                                                Some(&cookie_jar),
                                                is_over_cellular,
                                                context,
                                                gba_context,
                                                progress_callback,
                                            )
                                            .await;
                                        }
                                        Err(e) => {
                                            platform_log(
                                                LOG_TAG,
                                                format!("failed to receive otp with error {}", e),
                                            );
                                        }
                                    },
                                    Err(e) => {
                                        platform_log(
                                            LOG_TAG,
                                            format!("failed to receive otp in {}", e),
                                        );
                                    }
                                }
                            } else {
                                platform_log(
                                    LOG_TAG,
                                    "received 200 OK without either wap-doc or Set-Cookie header",
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
                                        connection.cipher_id(),
                                        GET,
                                        b"\"/\"",
                                        None,
                                        www_authenticate_header,
                                        &context.get_http_client(),
                                        &context.get_security_context(),
                                    )
                                    .await
                                    {
                                        return try_https_configuration_request(
                                            server,
                                            access_control_info,
                                            imei,
                                            imsi,
                                            msisdn,
                                            vers_version,
                                            token_string,
                                            cookie_jar,
                                            Some(&answer),
                                            is_over_cellular,
                                            context,
                                            gba_context,
                                            progress_callback,
                                        )
                                        .await;
                                    }
                                }
                            }
                        } else if resp.status_code == 403 {
                            if is_over_cellular {
                                return Err(DeviceConfigurationStatus::Forbidden);
                            } else {
                                return Err(DeviceConfigurationStatus::Retry(
                                    RetryReason::RequireCellularNetworkOrMsisdn,
                                    0,
                                ));
                            }
                        } else if resp.status_code == 404 {
                        } else if resp.status_code == 503 {
                            for header in HeaderSearch::new(&resp.headers, b"Retry-After", false) {
                                if let Ok(seconds) = header.get_value().to_int::<u32>() {
                                    return Err(DeviceConfigurationStatus::Retry(
                                        RetryReason::ServiceUnavailable,
                                        seconds,
                                    ));
                                }
                            }
                        } else if resp.status_code == 511 {
                            let path = format!("{}/{}", context.get_fs_root_dir(), imsi);
                            if let Some(provisioning_doc) = load_provisionging_doc(&path) {
                                let identifier = server.identifier();
                                let provisioning_doc = provisioning_doc.unlink_token(
                                    match server {
                                        ConfigurationServer::Default(_, _)
                                        | ConfigurationServer::PreConfiguredDefault(_, _, _) => {
                                            true
                                        }
                                        ConfigurationServer::Additional(_, _) => false,
                                    },
                                    &identifier,
                                );

                                if let Ok(_) = store_provisionging_doc(&provisioning_doc, &path) {
                                    platform_log(
                                        LOG_TAG,
                                        format!("provisioning-doc written to {}", &path),
                                    );
                                    return start_auto_config_for_server(
                                        server,
                                        access_control_info,
                                        imei,
                                        imsi,
                                        msisdn,
                                        context,
                                        gba_context,
                                        progress_callback,
                                    )
                                    .await;
                                }

                                return Err(DeviceConfigurationStatus::Retry(RetryReason::IO, 0));
                            }
                        }
                    }

                    Err(e) => {
                        platform_log(
                            LOG_TAG,
                            format!(
                                "try_https_configuration_request_inner send failed with error {:?}",
                                e
                            ),
                        );
                    }
                }
            }

            Err(e) => {
                platform_log(
                    LOG_TAG,
                    format!(
                        "try_https_configuration_request_inner connect failed with error {:?}",
                        e
                    ),
                );
            }
        }
    }

    Err(DeviceConfigurationStatus::Retry(RetryReason::NetworkIO, 0))
}

async fn try_initial_http_request(
    server: ConfigurationServer,
    access_control_info: Option<AccessControlInfo<'_>>,
    imei: &str,
    imsi: &str,
    msisdn: Option<&str>,
    vers_version: i64,
    token: Option<(&str, DateTime<FixedOffset>)>,
    context: Arc<Context>,
    gba_context: &GbaContext,
    progress_callback: Arc<Box<dyn Fn(i32) + Send + Sync + '_>>,
) -> Result<Option<String>, DeviceConfigurationStatus> {
    let http_uri = server.http_uri();

    if let Ok(url) = Url::parse(&http_uri) {
        if let Ok(connection) = context.get_http_client().connect(&url, false).await {
            let req = Request::new_with_default_headers(
                GET,
                url.host_str().unwrap(),
                url.path(),
                url.query(),
            );

            if let Ok((resp, _)) = connection.send(req, |_| {}).await {
                platform_log(
                    LOG_TAG,
                    format!(
                        "try_initial_http_request resp.status_code = {}",
                        resp.status_code
                    ),
                );

                if resp.status_code == 200 {
                    let mut cookie_jar = CookieJar::new();
                    for header in HeaderSearch::new(&resp.headers, b"Set-Cookie", false) {
                        if let Ok(cookie_string) = str::from_utf8(header.get_value()) {
                            if let Ok(cookie) = cookie_string.parse::<Cookie>() {
                                cookie_jar.add(cookie);
                            }
                        }
                    }
                    let is_over_cellular = cookie_jar.iter().count() > 0;
                    if let Some((token_string, token_validity_through)) = token {
                        if token_validity_through > Utc::now() {
                            return try_https_configuration_request(
                                server,
                                access_control_info,
                                imei,
                                imsi,
                                msisdn,
                                vers_version,
                                Some(token_string),
                                Some(&cookie_jar),
                                None,
                                is_over_cellular,
                                context,
                                gba_context,
                                progress_callback,
                            )
                            .await;
                        } else {
                            return try_https_configuration_request(
                                server,
                                access_control_info,
                                imei,
                                imsi,
                                msisdn,
                                vers_version,
                                None,
                                Some(&cookie_jar),
                                None,
                                is_over_cellular,
                                context,
                                gba_context,
                                progress_callback,
                            )
                            .await;
                        }
                    } else {
                        return try_https_configuration_request(
                            server,
                            access_control_info,
                            imei,
                            imsi,
                            msisdn,
                            vers_version,
                            None,
                            Some(&cookie_jar),
                            None,
                            is_over_cellular,
                            context,
                            gba_context,
                            progress_callback,
                        )
                        .await;
                    }
                } else if resp.status_code == 403 {
                    return Err(DeviceConfigurationStatus::Forbidden);
                } else if resp.status_code == 404 {
                } else if resp.status_code == 408 || resp.status_code == 511 {
                    if let Some((token_string, token_validity_through)) = token {
                        if token_validity_through > Utc::now() {
                            return try_https_configuration_request(
                                server,
                                access_control_info,
                                imei,
                                imsi,
                                msisdn,
                                vers_version,
                                Some(token_string),
                                None,
                                None,
                                false,
                                context,
                                gba_context,
                                progress_callback,
                            )
                            .await;
                        } else {
                            return try_https_configuration_request(
                                server,
                                access_control_info,
                                imei,
                                imsi,
                                msisdn,
                                vers_version,
                                None,
                                None,
                                None,
                                false,
                                context,
                                gba_context,
                                progress_callback,
                            )
                            .await;
                        }
                    } else {
                        return try_https_configuration_request(
                            server,
                            access_control_info,
                            imei,
                            imsi,
                            msisdn,
                            vers_version,
                            None,
                            None,
                            None,
                            false,
                            context,
                            gba_context,
                            progress_callback,
                        )
                        .await;
                    }
                }
            } else {
                platform_log(LOG_TAG, format!("try_initial_http_request failed"));
            }
        }
    }

    Err(DeviceConfigurationStatus::Retry(RetryReason::NetworkIO, 0))
}

async fn handle_config_for_server(
    server: ConfigurationServer,
    access_control_info: Option<AccessControlInfo<'_>>,
    imei: &str,
    imsi: &str,
    msisdn: Option<&str>,
    doc: &WapProvisioningDoc,
    context: Arc<Context>,
    gba_context: &GbaContext,
    progress_callback: Arc<Box<dyn Fn(i32) + Send + Sync + '_>>,
) -> Result<Option<String>, DeviceConfigurationStatus> {
    let mut provided_msisdn: Option<String> = None;
    match server {
        ConfigurationServer::Default(_, _) | ConfigurationServer::PreConfiguredDefault(_, _, _) => {
            platform_log(LOG_TAG, "finding access-control node for default server");
            if let Some(iter) = doc.access_control() {
                for access_control_info in iter {
                    if access_control_info.is_id_provider {
                        if access_control_info.app_ids().any(|app_id| {
                            app_id == "ap2001"
                                || app_id == "ap2002"
                                || app_id == "urn%3Aoma%3Amo%3Aext-3gpp-ims%3A1.0"
                        }) {
                            platform_log(
                                LOG_TAG,
                                format!("found additional server {}", access_control_info.fqdn),
                            );
                            let additional_server = ConfigurationServer::Additional(
                                String::from(access_control_info.fqdn),
                                true,
                            );
                            match start_auto_config_for_server(
                                additional_server,
                                Some(access_control_info),
                                imei,
                                imsi,
                                msisdn,
                                Arc::clone(&context),
                                gba_context,
                                Arc::clone(&progress_callback),
                            )
                            .await
                            {
                                Ok(possible_user_msisdn) => {
                                    if let Some(user_msisdn) = possible_user_msisdn {
                                        provided_msisdn.replace(user_msisdn);
                                    }
                                }
                                Err(e) => {
                                    return Err(e);
                                }
                            }
                        }
                    }
                }
            }

            if let Some(iter) = doc.access_control() {
                for access_control_info in iter {
                    if !access_control_info.is_id_provider {
                        if access_control_info.app_ids().any(|app_id| {
                            app_id == "ap2001"
                                || app_id == "ap2002"
                                || app_id == "urn%3Aoma%3Amo%3Aext-3gpp-ims%3A1.0"
                        }) {
                            platform_log(
                                LOG_TAG,
                                format!("found additional server {}", access_control_info.fqdn),
                            );
                            let additional_server = ConfigurationServer::Additional(
                                String::from(access_control_info.fqdn),
                                false,
                            );
                            if let Some(ref provided_msisdn) = provided_msisdn {
                                start_auto_config_for_server(
                                    additional_server,
                                    Some(access_control_info),
                                    imei,
                                    imsi,
                                    Some(provided_msisdn),
                                    Arc::clone(&context),
                                    gba_context,
                                    Arc::clone(&progress_callback),
                                )
                                .await?;
                            } else {
                                start_auto_config_for_server(
                                    additional_server,
                                    Some(access_control_info),
                                    imei,
                                    imsi,
                                    msisdn,
                                    Arc::clone(&context),
                                    gba_context,
                                    Arc::clone(&progress_callback),
                                )
                                .await?;
                            }
                        }
                    }
                }
            }
        }

        ConfigurationServer::Additional(_, is_id_provider) => {
            if is_id_provider {
                if let Some(user_msisdn) = doc.get_user_msisdn() {
                    provided_msisdn.replace(String::from(user_msisdn));
                }
            }
        }
    }

    let default: &str = match server {
        ConfigurationServer::Default(_, _) | ConfigurationServer::PreConfiguredDefault(_, _, _) => {
            "yes"
        }
        ConfigurationServer::Additional(_, _) => "no",
    };

    let identifier = server.identifier();

    platform_log(
        LOG_TAG,
        format!(
            "handle wap provisioning doc for server {} --is-default?:{}",
            &identifier, default
        ),
    );

    if let Some((vers, version_validity, token)) = doc.get_vers_token() {
        let vers_validity_through;
        if vers > 0 {
            vers_validity_through = (Utc::now() + Duration::seconds(version_validity)).to_rfc3339();
        } else {
            vers_validity_through = (Utc::now() + Duration::days(365)).to_rfc3339();
        }

        if vers > 0 {
            let path = format!("{}/{}", context.fs_root_dir, imsi);
            if let Some(mut provisioning_doc) = load_provisionging_doc(&path) {
                platform_log(LOG_TAG, "update existing provisioning-doc");

                provisioning_doc.update_vers_token(
                    default == "yes",
                    &identifier,
                    vers,
                    vers_validity_through,
                    token,
                );

                for application in doc.applications() {
                    platform_log(LOG_TAG, "processing application in wap provisioning doc");
                    if let Some(app_id) = application.get_parameter("AppID") {
                        platform_log(LOG_TAG, format!("trying to insert application {}", app_id));
                        if let Some(ref access_control_info) = access_control_info {
                            if !access_control_info
                                .app_ids()
                                .any(|app_id_| app_id_ == app_id)
                            {
                                platform_log(LOG_TAG, "forbidden by access_control");
                                continue;
                            }
                        }

                        provisioning_doc = provisioning_doc.update_application(
                            default == "yes",
                            &identifier,
                            app_id,
                            &application,
                        );
                    }
                }

                if default == "yes" {
                    provisioning_doc.remove_non_application_characteristics_for_default_server();
                    for characteristic in doc.children() {
                        if characteristic.characteristic_type != "APPLICATION" {
                            provisioning_doc
                                .update_characteristics_for_default_server(characteristic);
                        }
                    }
                }

                if let Ok(_) = store_provisionging_doc(&provisioning_doc, &path) {
                    platform_log(LOG_TAG, format!("provisioning-doc written to {}", &path));
                    return Ok(provided_msisdn);
                }

                return Err(DeviceConfigurationStatus::Retry(RetryReason::IO, 0));
            } else {
                platform_log(LOG_TAG, "creating new provisioning-doc");

                let mut v = Vec::new();

                for characteristic in doc.children() {
                    if characteristic.characteristic_type == "APPLICATION" {
                        platform_log(LOG_TAG, "processing applications in wap provisioning doc");
                        if let Some(app_id) = characteristic.get_parameter("AppID") {
                            platform_log(
                                LOG_TAG,
                                format!("trying to insert application {}", app_id),
                            );
                            if let Some(ref access_control_info) = access_control_info {
                                if !access_control_info
                                    .app_ids()
                                    .any(|app_id_| app_id_ == app_id)
                                {
                                    platform_log(LOG_TAG, "forbidden by access_control");
                                    continue;
                                }
                            }

                            v.push(characteristic.clone());
                        }
                    } else {
                        if default == "yes" {
                            v.push(characteristic.clone())
                        }
                    }
                }

                let wap_provisioning_doc = WapProvisioningDoc::new(v);

                let mut provisioning_info = ProvisiongingInfo {
                    default: String::from(default),
                    id: identifier,
                    VERS_version: format!("{}", vers),
                    VERS_validity_through: vers_validity_through,
                    TOKEN_token: None,
                    TOKEN_validity_through: None,
                    wap_doc: Some(wap_provisioning_doc),
                };

                if let Some((token_string, token_validity)) = token {
                    provisioning_info
                        .TOKEN_token
                        .replace(String::from(token_string));

                    if let Some(token_validity) = token_validity {
                        if token_validity > 0 {
                            let token_validity_through =
                                (Utc::now() + Duration::seconds(token_validity)).to_rfc3339();
                            provisioning_info
                                .TOKEN_validity_through
                                .replace(token_validity_through);
                        } else {
                            let token_validity_through =
                                (Utc::now() + Duration::days(365)).to_rfc3339();
                            provisioning_info
                                .TOKEN_validity_through
                                .replace(token_validity_through);
                        }
                    } else {
                        let token_validity_through =
                            (Utc::now() + Duration::days(365)).to_rfc3339();
                        provisioning_info
                            .TOKEN_validity_through
                            .replace(token_validity_through);
                    }
                }

                let mut v = Vec::new();

                v.push(provisioning_info);

                let provisioning_doc = LocalProvisioningDoc::new(v);

                if let Ok(_) = store_provisionging_doc(&provisioning_doc, &path) {
                    platform_log(LOG_TAG, format!("provisioning-doc written to {}", &path));
                    return Ok(provided_msisdn);
                }

                return Err(DeviceConfigurationStatus::Retry(RetryReason::IO, 0));
            }
        }

        if vers <= 0 {
            let path = format!("{}/{}", context.fs_root_dir, imsi);
            let mut provisioning_doc = load_provisionging_doc(&path);
            if let Some(mut doc) = provisioning_doc.take() {
                doc = doc.unlink(default == "yes", &identifier);
                provisioning_doc.replace(doc);
            }

            if vers < 0 {
                if provisioning_doc.is_none() {
                    let provisioning_info = ProvisiongingInfo {
                        default: String::from(default),
                        id: identifier,
                        VERS_version: format!("{}", vers),
                        VERS_validity_through: String::from(vers_validity_through),
                        TOKEN_token: None,
                        TOKEN_validity_through: None,
                        wap_doc: None,
                    };

                    let mut v = Vec::new();

                    v.push(provisioning_info);

                    provisioning_doc.replace(LocalProvisioningDoc::new(v));
                }
            }

            if let Some(provisioning_doc) = provisioning_doc {
                if let Ok(_) = store_provisionging_doc(&provisioning_doc, &path) {
                    platform_log(LOG_TAG, format!("provisioning-doc written to {}", &path));
                    return Ok(provided_msisdn);
                }

                return Err(DeviceConfigurationStatus::Retry(RetryReason::IO, 0));
            }
        }
    }

    Ok(provided_msisdn)
}

fn start_auto_config_for_server<'a, 'b: 'a>(
    server: ConfigurationServer,
    access_control_info: Option<AccessControlInfo<'a>>,
    imei: &'b str,
    imsi: &'b str,
    msisdn: Option<&'b str>,
    context: Arc<Context>,
    gba_context: &'a GbaContext,
    progress_callback: Arc<Box<dyn Fn(i32) + Send + Sync + 'b>>,
) -> BoxFuture<'a, Result<Option<String>, DeviceConfigurationStatus>> {
    async move {
        start_auto_config_for_server_inner(
            server,
            access_control_info,
            imei,
            imsi,
            msisdn,
            context,
            gba_context,
            progress_callback,
        )
        .await
    }
    .boxed()
}

async fn start_auto_config_for_server_inner(
    server: ConfigurationServer,
    access_control_info: Option<AccessControlInfo<'_>>,
    imei: &str,
    imsi: &str,
    msisdn: Option<&str>,
    context: Arc<Context>,
    gba_context: &GbaContext,
    progress_callback: Arc<Box<dyn Fn(i32) + Send + Sync + '_>>,
) -> Result<Option<String>, DeviceConfigurationStatus> {
    let path = format!("{}/{}", context.fs_root_dir, imsi);
    platform_log(LOG_TAG, format!("loading provisioning doc from {}", &path));
    if let Some(provisioning_doc) = load_provisionging_doc(&path) {
        platform_log(LOG_TAG, "local provisioning doc loaded");

        if let Some((vers_version, vers_validity_through, token, doc)) = match &server {
            ConfigurationServer::Default(_, _)
            | ConfigurationServer::PreConfiguredDefault(_, _, _) => {
                let identifier = server.identifier();
                platform_log(
                    LOG_TAG,
                    format!("get provisioning info with default server {}", &identifier),
                );
                provisioning_doc.get_provisioning_info(true, &identifier)
            }
            ConfigurationServer::Additional(fqdn, _) => {
                platform_log(LOG_TAG, format!("get provisioning info with fqdn {}", fqdn));
                provisioning_doc.get_provisioning_info(false, fqdn)
            }
        } {
            platform_log(
                LOG_TAG,
                format!(
                    "VERS_version {:?} VERS_validity_through {:?}",
                    vers_version, vers_validity_through
                ),
            );

            if vers_version < 0 {
                return Err(DeviceConfigurationStatus::Forbidden);
            }

            if vers_validity_through > Utc::now() {
                if let Some(doc) = doc {
                    return handle_config_for_server(
                        server,
                        access_control_info,
                        imei,
                        imsi,
                        msisdn,
                        doc,
                        context,
                        gba_context,
                        progress_callback,
                    )
                    .await;
                }
            }

            return try_initial_http_request(
                server,
                access_control_info,
                imei,
                imsi,
                msisdn,
                0,
                token,
                context,
                gba_context,
                progress_callback,
            )
            .await;
        }
    }

    try_initial_http_request(
        server,
        access_control_info,
        imei,
        imsi,
        msisdn,
        0,
        None,
        context,
        gba_context,
        progress_callback,
    )
    .await
}

pub async fn start_auto_config<F>(
    subscription_id: i32,
    mcc: CountryCode,
    mnc: NetworkCode,
    imei: &str,
    imsi: &str,
    msisdn: Option<&str>,
    context: Arc<Context>,
    progress_callback: F,
) -> Result<(Option<String>, Characteristic, Characteristic), DeviceConfigurationStatus>
where
    F: Fn(i32) + Send + Sync + 'static,
{
    if mcc.is_valid_three_digits() && mnc.is_valid_three_digits() {
        let server = ConfigurationServer::Default(mcc, mnc);
        let gba_context = context.make_gba_context(imsi, mcc, mnc, subscription_id);
        let provided_msisdn = start_auto_config_for_server(
            server,
            None,
            imei,
            imsi,
            msisdn,
            Arc::clone(&context),
            &gba_context,
            Arc::new(Box::new(progress_callback)),
        )
        .await?;
        platform_log(
            LOG_TAG,
            format!("complete with provided_msisdn {:?}", provided_msisdn),
        );
        let path = format!("{}/{}", &context.fs_root_dir, imsi);
        if let Some(provisioning_doc) = load_provisionging_doc(&path) {
            for doc in provisioning_doc.provisioning_files() {
                if let Some(rcs_application) = doc.get_rcs_application() {
                    let to_app_ref = rcs_application.get_to_app_ref();
                    for doc in provisioning_doc.provisioning_files() {
                        if let Some(ims_application) = doc.get_ims_application(to_app_ref) {
                            return Ok((
                                provided_msisdn,
                                rcs_application.clone_characteristic(),
                                ims_application.clone_characteristic(),
                            ));
                        }
                    }
                }
            }
        }
        return Err(DeviceConfigurationStatus::Forbidden);
    } else {
        Err(DeviceConfigurationStatus::InvalidArgument)
    }
}
