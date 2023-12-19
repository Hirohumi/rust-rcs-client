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

pub mod session;
pub mod session_invitation;
pub mod sip;
pub mod standalone_messaging;

use std::sync::Arc;

use base64::{engine::general_purpose, Engine as _};
use chrono::{DateTime, Local, SecondsFormat};
use rust_rcs_core::{
    internet::{body::message_body::MessageBody, Body, Header},
    sip::sip_session::SipSession,
};
use uuid::Uuid;

use self::session::CPMSession;

use super::ffi::RecipientType;

pub struct MessagingSessionHandle {
    pub(crate) inner: Arc<SipSession<CPMSession>>,
}

pub struct CPMMessageParam {
    message_type: String,
    message_content: String,
    recipient: String,
    recipient_type: RecipientType,
    recipient_uri: String,
    message_result_callback: Arc<dyn Fn(u16, String) + Send + Sync>, // to-do: use FnOnce in CPMSession ?
}

impl CPMMessageParam {
    pub fn new<F>(
        message_type: String,
        message_content: String,
        recipient: &str,
        recipient_type: &RecipientType,
        recipient_uri: &str,
        message_result_callback: F,
    ) -> CPMMessageParam
    where
        F: Fn(u16, String) + Send + Sync + 'static,
    {
        CPMMessageParam {
            message_type,
            message_content,
            recipient: String::from(recipient),
            recipient_type: RecipientType::from(recipient_type),
            recipient_uri: String::from(recipient_uri),
            message_result_callback: Arc::new(message_result_callback),
        }
    }
}

pub fn make_cpim_message_content_body(
    message_type: &str,
    message_content: &str,
    recipient_type: &RecipientType,
    recipient_uri: &str,
    message_imdn_id: Uuid,
    public_user_identity: &str,
) -> Body {
    let encoded = general_purpose::STANDARD.encode(message_content);

    let content_body = Vec::from(encoded);
    let content_body_length = content_body.len();

    let mut content_headers = Vec::new();

    content_headers.push(Header::new("Content-Type", String::from(message_type)));

    content_headers.push(Header::new("Content-Transfer-Encoding", "base64"));

    content_headers.push(Header::new(
        "Content-Length",
        format!("{}", content_body_length),
    ));

    let cpim_content_body = Body::Message(MessageBody {
        headers: content_headers,
        body: Arc::new(Body::Raw(content_body)),
    });

    let mut cpim_headers = Vec::new();

    if message_type.eq("message/imdn") {
        cpim_headers.push(Header::new("From", "<sip:anonymous@anonymous.invalid>"));
        cpim_headers.push(Header::new("To", "<sip:anonymous@anonymous.invalid>"));
    } else {
        cpim_headers.push(Header::new("From", format!("<{}>", public_user_identity)));
        cpim_headers.push(Header::new("To", format!("<{}>", recipient_uri)));
    }

    let utc: DateTime<Local> = Local::now();

    cpim_headers.push(Header::new(
        "DateTime",
        utc.to_rfc3339_opts(SecondsFormat::Millis, false),
    ));

    cpim_headers.push(Header::new("NS", "imdn <urn:ietf:params:imdn>"));

    cpim_headers.push(Header::new(
        "imdn.Message-ID",
        String::from(
            message_imdn_id
                .as_hyphenated()
                .encode_lower(&mut Uuid::encode_buffer()),
        ),
    ));

    // if !message_type.eq("message/imdn") {
    //     cpim_headers.push(Header::new(
    //         "imdn.Disposition-Notification",
    //         "positive-delivery",
    //     ));
    // }

    cpim_headers.push(Header::new(
        "NS",
        "cpm <http://www.openmobilealliance.org/cpm/>",
    ));

    cpim_headers.push(Header::new(b"cpm.Payload-Type", String::from(message_type)));

    // cpim_headers.push(Header {
    //     name: b"NS".to_vec(),
    //     value: b"cpim <http://www.openmobilealliance.org/cpm/cpim>".to_vec(),
    // });

    if let RecipientType::Chatbot = recipient_type {
        cpim_headers.push(Header::new("NS", "maap <http://www.gsma.com/rcs/maap/>"));
    }

    Body::Message(MessageBody {
        headers: cpim_headers,
        body: Arc::new(cpim_content_body),
    })
}
