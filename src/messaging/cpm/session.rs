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

extern crate rust_rcs_core;
extern crate rust_strict_sdp;

use std::io::Read;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose, Engine as _};
use futures::Future;

use rust_rcs_core::cpim::{CPIMInfo, CPIMMessage};
use rust_rcs_core::internet::body::VectorReader;
use rust_rcs_core::internet::header::search;
use rust_rcs_core::internet::header_field::AsHeaderField;
use rust_rcs_core::internet::Body;
use rust_rcs_core::internet::{header, Header};

use rust_rcs_core::internet::headers::{AsContentType, Supported};

use rust_rcs_core::internet::name_addr::AsNameAddr;
use rust_rcs_core::io::network::stream::{ClientSocket, ClientStream};
use rust_rcs_core::io::{DynamicChain, Serializable};
use rust_rcs_core::msrp::info::msrp_info_reader::AsMsrpInfo;
use rust_rcs_core::msrp::info::{MsrpDirection, MsrpInfo, MsrpInterfaceType, MsrpSetupMethod};
use rust_rcs_core::msrp::msrp_channel::MsrpChannel;
use rust_rcs_core::msrp::msrp_chunk::MsrpChunk;
use rust_rcs_core::msrp::msrp_demuxer::MsrpDemuxer;
use rust_rcs_core::msrp::msrp_muxer::{MsrpDataWriter, MsrpMessageReceiver, MsrpMuxer};
use rust_rcs_core::msrp::msrp_transport::msrp_transport_start;
// use rust_rcs_core::msrp::msrp_transport::MsrpTransportWrapper;
use rust_rcs_core::sip::sip_core::SipDialogCache;
use rust_rcs_core::sip::sip_session::{
    choose_timeout_for_server_transaction_response,
    choose_timeout_on_client_transaction_completion, Refresher, SipSession, SipSessionEvent,
    SipSessionEventReceiver,
};
use rust_rcs_core::sip::sip_transaction::client_transaction::ClientTransactionNilCallbacks;
use rust_rcs_core::sip::sip_transaction::server_transaction;

use rust_rcs_core::sip::SipDialog;
use rust_rcs_core::sip::{ClientTransactionCallbacks, SipTransactionManager};
use rust_rcs_core::sip::{ServerTransaction, SipDialogEventCallbacks};
use rust_rcs_core::sip::{ServerTransactionEvent, UPDATE};
use rust_rcs_core::sip::{SipCore, SipTransport};

use rust_rcs_core::sip::SipMessage;
use rust_rcs_core::sip::TransactionHandler;

use rust_rcs_core::sip::INVITE;

use rust_rcs_core::sip::sip_headers::AsFromTo;

use rust_rcs_core::util::rand::create_raw_alpha_numeric_string;
use rust_rcs_core::util::raw_string::StrFind;

use rust_strict_sdp::AsSDP;

use tokio::runtime::Runtime;
use tokio::sync::mpsc;
use tokio::sync::oneshot;
use uuid::Uuid;

use crate::contact::ContactKnownIdentities;
use crate::messaging::ffi::RecipientType;

use super::session_invitation::{
    try_accept_hanging_invitation, try_accept_invitation, CPMSessionInvitation,
    CPMSessionInvitationResponse,
};
use super::sip::cpm_accept_contact::CPMAcceptContact;
use super::sip::cpm_contact::{AsCPMContact, CPMContact, CPMServiceType};
use super::{make_cpim_message_content_body, CPMMessageParam};

// to-do: use enum
pub struct CPMSessionInfo {
    pub cpm_contact: CPMContact,
    pub asserted_contact_uri: String,
    pub conversation_id: Option<String>,
    pub contribution_id: Option<String>,
    pub subject: Option<String>,
    pub referred_by: Option<String>,
    pub is_deferred_session: bool,
    pub contact_alias: Option<String>,
    pub conversation_supports_anonymization: bool,
    pub conversation_is_using_anonymization_token: bool,
}

impl CPMSessionInfo {
    pub fn get_contact_known_identities(&self) -> ContactKnownIdentities {
        let mut known_identities = ContactKnownIdentities::new();
        known_identities.add_identity(self.asserted_contact_uri.clone());
        match self.cpm_contact.service_type {
            CPMServiceType::OneToOne | CPMServiceType::Chatbot => {
                if let Ok(service_uri) = std::str::from_utf8(&self.cpm_contact.service_uri) {
                    known_identities.add_identity(String::from(service_uri));
                }
            }
            _ => {}
        }
        if let Some(contact_alias) = &self.contact_alias {
            known_identities.add_identity(String::from(contact_alias));
        }
        known_identities
    }
}

pub fn get_cpm_session_info_from_message(
    message: &SipMessage,
    is_outgoing: bool,
) -> Option<CPMSessionInfo> {
    if let Some(headers) = message.headers() {
        let mut contact_header = None;
        let mut asserted_identity_header = None;
        let mut subject_header = None;
        let mut referred_by_header = None;

        let mut from_header = None;
        let mut to_header = None;

        let mut conversation_id: Option<&[u8]> = None;
        let mut contribution_id: Option<&[u8]> = None;

        for header in headers {
            if header.get_name().eq_ignore_ascii_case(b"Contact") {
                contact_header = Some(header);
            } else if header
                .get_name()
                .eq_ignore_ascii_case(b"P-Asserted-Identity")
            {
                asserted_identity_header = Some(header);
            } else if header.get_name().eq_ignore_ascii_case(b"Conversation-ID") {
                conversation_id = Some(header.get_value());
            } else if header.get_name().eq_ignore_ascii_case(b"Contribution-ID") {
                contribution_id = Some(header.get_value());
            } else if header.get_name().eq_ignore_ascii_case(b"Subject") {
                subject_header = Some(header);
            } else if header.get_name().eq_ignore_ascii_case(b"Referred-By") {
                referred_by_header = Some(header);
            } else if header.get_name().eq_ignore_ascii_case(b"From") {
                from_header = Some(header);
            } else if header.get_name().eq_ignore_ascii_case(b"To") {
                to_header = Some(header);
            }
        }

        if let Some(contact_header) = contact_header {
            if let Some(cpm_contact) = contact_header
                .get_value()
                .as_header_field()
                .as_cpm_contact()
            {
                let mut asserted_contact_uri = None;
                let mut is_deferred_session = false;
                let mut conversation_supports_anonymization = false;
                let mut conversation_is_using_anonymization_token = false;

                match cpm_contact.service_type {
                    CPMServiceType::OneToOne | CPMServiceType::Chatbot => {
                        if let Some(asserted_identity_header) = asserted_identity_header {
                            if let Some(name_addr) = asserted_identity_header
                                .get_value()
                                .as_name_addresses()
                                .first()
                            {
                                if let Some(uri_part) = &name_addr.uri_part {
                                    if uri_part.uri.start_with(b"rcse-standfw@") {
                                        is_deferred_session = true;
                                        if let Some(referred_by_header) = referred_by_header {
                                            if let Some(referred_by) = referred_by_header
                                                .get_value()
                                                .as_name_addresses()
                                                .first()
                                            {
                                                if let Some(uri_part) = &referred_by.uri_part {
                                                    asserted_contact_uri = Some(uri_part.uri);
                                                    for p in referred_by_header
                                                        .get_value()
                                                        .as_header_field()
                                                        .get_parameter_iterator()
                                                    {
                                                        if p.name == b"tk" {
                                                            match p.value {
                                                                Some(b"on") => {
                                                                    conversation_supports_anonymization = true;
                                                                    conversation_is_using_anonymization_token = true;
                                                                }
                                                                Some(b"off") => {
                                                                    conversation_supports_anonymization = true;
                                                                }
                                                                _ => {}
                                                            }
                                                            break;
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }

                                    if !is_deferred_session {
                                        asserted_contact_uri = Some(uri_part.uri);
                                        for p in uri_part.get_parameter_iterator() {
                                            if p.name == b"tk" {
                                                match p.value {
                                                    Some(b"on") => {
                                                        conversation_supports_anonymization = true;
                                                        conversation_is_using_anonymization_token =
                                                            true;
                                                    }
                                                    Some(b"off") => {
                                                        conversation_supports_anonymization = true;
                                                    }
                                                    _ => {}
                                                }
                                                break;
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }

                    _ => {
                        if let Some(asserted_identity_header) = asserted_identity_header {
                            if let Some(name_addr) = asserted_identity_header
                                .get_value()
                                .as_name_addresses()
                                .first()
                            {
                                if let Some(uri_part) = &name_addr.uri_part {
                                    asserted_contact_uri = Some(uri_part.uri);
                                }
                            }
                        }
                    }
                }

                if asserted_contact_uri.is_none() {
                    asserted_contact_uri = Some(&cpm_contact.service_uri);
                }

                if let Some(asserted_contact_uri) = asserted_contact_uri {
                    if let Ok(asserted_contact_uri) = std::str::from_utf8(asserted_contact_uri) {
                        let asserted_contact_uri = String::from(asserted_contact_uri);

                        let mut contact_alias = None;
                        if is_outgoing {
                            if let Some(to_header) = to_header {
                                if let Some(name_addr) = to_header
                                    .get_value()
                                    .as_header_field()
                                    .as_from_to()
                                    .addresses
                                    .first()
                                {
                                    if let Some(display_name) = name_addr.display_name {
                                        if let Ok(display_name) = std::str::from_utf8(display_name)
                                        {
                                            contact_alias = Some(String::from(display_name));
                                        }
                                    }
                                }
                            }
                        } else {
                            if let Some(from_header) = from_header {
                                if let Some(name_addr) = from_header
                                    .get_value()
                                    .as_header_field()
                                    .as_from_to()
                                    .addresses
                                    .first()
                                {
                                    if let Some(display_name) = name_addr.display_name {
                                        if let Ok(display_name) = std::str::from_utf8(display_name)
                                        {
                                            contact_alias = Some(String::from(display_name));
                                        }
                                    }
                                }
                            }
                        }

                        return Some(CPMSessionInfo {
                            cpm_contact,
                            asserted_contact_uri,
                            conversation_id: match conversation_id {
                                Some(conversation_id) => match std::str::from_utf8(conversation_id)
                                {
                                    Ok(conversation_id) => Some(String::from(conversation_id)),
                                    Err(_) => None,
                                },
                                None => None,
                            },
                            contribution_id: match contribution_id {
                                Some(contribution_id) => match std::str::from_utf8(contribution_id)
                                {
                                    Ok(contribution_id) => Some(String::from(contribution_id)),
                                    Err(_) => None,
                                },
                                None => None,
                            },
                            subject: match subject_header {
                                Some(subject_header) => {
                                    if let Ok(subject) =
                                        std::str::from_utf8(subject_header.get_value())
                                    {
                                        Some(String::from(subject))
                                    } else {
                                        None
                                    }
                                }
                                None => None,
                            },
                            referred_by: match referred_by_header {
                                Some(referred_by_header) => {
                                    if let Ok(referred_by) =
                                        std::str::from_utf8(referred_by_header.get_value())
                                    {
                                        Some(String::from(referred_by))
                                    } else {
                                        None
                                    }
                                }
                                None => None,
                            },
                            is_deferred_session,
                            contact_alias,
                            conversation_supports_anonymization,
                            conversation_is_using_anonymization_token,
                        });
                    }
                }
            }
        }
    }

    None
}

enum CPMSessionNegotiationState {
    NoneSet,
    LocalSet(Arc<Body>, Option<ClientSocket>, bool, String, u16, Vec<u8>),
    RemoteSet(Arc<Body>, String, u16, Vec<u8>),
}

enum CPMSessionState {
    Negotiating(CPMSessionNegotiationState),
    Negotiated(
        Arc<Body>,
        Arc<Body>,
        Option<ClientSocket>,
        bool,
        String,
        u16,
        Vec<u8>,
        String,
        u16,
        Vec<u8>,
    ),
    Starting(
        Arc<Body>,
        Arc<Body>,
        String,
        u16,
        String,
        u16,
        // Arc<MsrpChannel>,
        mpsc::Sender<CPMMessageParam>,
    ),
    // Started(
    //     Arc<Body>,
    //     Arc<Body>,
    //     String,
    //     u16,
    //     String,
    //     u16,
    //     Arc<MsrpTransport>,
    // ),
    // also
    //  there is no Re-negotiating for CPM Session
}

pub struct CPMSession {
    session_info: CPMSessionInfo,
    session_state: Arc<Mutex<CPMSessionState>>,
}

impl CPMSession {
    pub fn new(session_info: CPMSessionInfo) -> CPMSession {
        CPMSession {
            session_info,
            session_state: Arc::new(Mutex::new(CPMSessionState::Negotiating(
                CPMSessionNegotiationState::NoneSet,
            ))),
        }
    }

    pub fn get_session_info(&self) -> &CPMSessionInfo {
        &self.session_info
    }

    pub fn set_local_sdp(
        &self,
        l_sdp: Arc<Body>,
        cs: ClientSocket,
        tls: bool,
        laddr: String,
        lport: u16,
        lpath: Vec<u8>,
    ) -> Result<(), &'static str> {
        let mut guard = self.session_state.lock().unwrap();
        match &mut *guard {
            CPMSessionState::Negotiating(ref mut negotiation_state) => match negotiation_state {
                CPMSessionNegotiationState::NoneSet => {
                    *guard = CPMSessionState::Negotiating(CPMSessionNegotiationState::LocalSet(
                        l_sdp,
                        Some(cs),
                        tls,
                        laddr,
                        lport,
                        lpath,
                    ));
                    Ok(())
                }
                CPMSessionNegotiationState::LocalSet(_, _, _, _, _, _) => Err("local already set"),
                CPMSessionNegotiationState::RemoteSet(
                    ref r_sdp,
                    ref raddr,
                    ref rport,
                    ref mut rpath,
                ) => {
                    let mut path = Vec::new();
                    path.append(rpath);
                    *guard = CPMSessionState::Negotiated(
                        l_sdp,
                        Arc::clone(r_sdp),
                        Some(cs),
                        tls,
                        laddr,
                        lport,
                        lpath,
                        String::from(raddr),
                        *rport,
                        path,
                    );
                    Ok(())
                }
            },

            _ => Err("already negotiated"),
        }
    }

    pub fn set_remote_sdp(
        &self,
        r_sdp: Arc<Body>,
        raddr: String,
        rport: u16,
        rpath: Vec<u8>,
    ) -> Result<(), &'static str> {
        let mut guard = self.session_state.lock().unwrap();
        match &mut *guard {
            CPMSessionState::Negotiating(ref mut negotiation_state) => match negotiation_state {
                CPMSessionNegotiationState::NoneSet => {
                    *guard = CPMSessionState::Negotiating(CPMSessionNegotiationState::RemoteSet(
                        r_sdp, raddr, rport, rpath,
                    ));
                    Ok(())
                }
                CPMSessionNegotiationState::LocalSet(
                    ref l_sdp,
                    ref mut sock,
                    ref tls,
                    ref laddr,
                    ref lport,
                    ref mut lpath,
                ) => {
                    let mut path = Vec::new();
                    path.append(lpath);
                    *guard = CPMSessionState::Negotiated(
                        Arc::clone(l_sdp),
                        r_sdp,
                        sock.take(),
                        *tls,
                        String::from(laddr),
                        *lport,
                        path,
                        raddr,
                        rport,
                        rpath,
                    );
                    Ok(())
                }
                CPMSessionNegotiationState::RemoteSet(_, _, _, _) => Err("remote already set"),
            },

            _ => Err("already negotiated"),
        }
    }

    pub fn start<MRL, F>(
        &self,
        public_user_identity: &str,
        message_receive_listener: MRL,
        connect_function: &Arc<
            dyn Fn(
                    ClientSocket,
                    &String,
                    u16,
                    bool,
                ) -> Pin<
                    Box<dyn Future<Output = Result<ClientStream, (u16, &'static str)>> + Send>,
                > + Send
                + Sync
                + 'static,
        >,
        session_end_function: F,
        rt: &Arc<Runtime>,
    ) -> Result<mpsc::Sender<CPMMessageParam>, ()>
    where
        MRL: Fn(&CPIMInfo, &[u8], &[u8]) + Send + Sync + 'static,
        F: Fn() + Send + Sync + 'static,
    {
        let mut guard = self.session_state.lock().unwrap();
        match &mut *guard {
            CPMSessionState::Negotiated(
                ref l_sdp,
                ref r_sdp,
                ref mut sock,
                ref tls,
                ref laddr,
                ref lport,
                ref mut lpath,
                ref raddr,
                ref rport,
                ref mut rpath,
            ) => {
                if let Some(sock) = sock.take() {
                    let mut from_path = Vec::with_capacity(lpath.len());
                    from_path.append(lpath);
                    let mut to_path = Vec::with_capacity(rpath.len());
                    to_path.append(rpath);
                    let from_path_ = from_path.clone();
                    let to_path_ = to_path.clone();
                    let demuxer = MsrpDemuxer::new(from_path_, to_path_);
                    let demuxer = Arc::new(demuxer);
                    let demuxer_ = Arc::clone(&demuxer);
                    let message_receive_listener = Arc::new(message_receive_listener);
                    let muxer = MsrpMuxer::new(CPMSessionMessageReceiver {
                        callback: Arc::new(move |cpim_info, content_type, content_body| {
                            message_receive_listener(cpim_info, content_type, content_body)
                        }),
                    });

                    let from_path_ = from_path.clone();
                    let to_path_ = to_path.clone();

                    let mut session_channel =
                        MsrpChannel::new(from_path_, to_path_, demuxer, muxer);
                    // let session_channel = Arc::new(session_channel);
                    // let session_channel_ = Arc::clone(&session_channel);

                    let addr = String::from(raddr);
                    let port = *rport;
                    let tls = *tls;

                    let l_sdp = Arc::clone(l_sdp);
                    // let l_sdp_ = Arc::clone(&l_sdp);
                    let r_sdp = Arc::clone(r_sdp);
                    // let r_sdp_ = Arc::clone(&r_sdp);

                    let laddr = String::from(laddr);
                    // let laddr_ = String::from(&laddr);
                    let lport = *lport;
                    let raddr = String::from(raddr);
                    // let raddr_ = String::from(&raddr);
                    let rport = *rport;

                    let (message_tx, mut message_rx) = mpsc::channel::<CPMMessageParam>(8);
                    let message_tx_ = message_tx.clone();

                    *guard = CPMSessionState::Starting(
                        l_sdp, r_sdp, laddr, lport, raddr, rport,
                        // session_channel,
                        message_tx,
                    );

                    let t_id = String::from(public_user_identity);
                    let public_user_identity = String::from(public_user_identity);

                    let rt_ = Arc::clone(rt);

                    // let session_state = Arc::clone(&self.session_state);

                    let future = connect_function(sock, &addr, port, tls);

                    rt.spawn(async move {
                        if let Ok(cs) = future.await {
                            // let transport = MsrpTransportWrapper::new(
                            //     cs,
                            //     t_id,
                            //     move |message_chunk| session_channel.on_message(message_chunk),
                            //     move |transport| session_end_function(),
                            //     &rt_,
                            // );

                            // let t = transport.get_transport();

                            // let mut guard = session_state.lock().unwrap();

                            // *guard = CPMSessionState::Started(l_sdp_, r_sdp_, laddr_, lport, raddr_, rport, t);

                            let (data_tx, data_rx) = mpsc::channel(8);
                            let data_tx_ = data_tx.clone();

                            msrp_transport_start(
                                cs,
                                t_id,
                                data_tx_,
                                data_rx,
                                move |message_chunk| session_channel.on_message(message_chunk),
                                move || session_end_function(),
                                &rt_,
                            );

                            let data_tx_ = data_tx.clone();

                            let (chunk_tx, mut chunk_rx) = mpsc::channel::<MsrpChunk>(16);

                            rt_.spawn(async move {
                                while let Some(chunk) = chunk_rx.recv().await {
                                    let size = chunk.estimated_size();
                                    let mut data = Vec::with_capacity(size);

                                    {
                                        let mut readers = Vec::new();
                                        chunk.get_readers(&mut readers);
                                        match DynamicChain::new(readers).read_to_end(&mut data) {
                                            Ok(_) => {}
                                            Err(_) => {} // to-do: early failure
                                        }
                                    }

                                    match data_tx_.send(Some(data)).await {
                                        Ok(()) => {}
                                        Err(e) => {}
                                    }
                                }
                            });

                            while let Some(message) = message_rx.recv().await {
                                let content_type = b"message/cpim".to_vec();
                                let content_type = Some(content_type);

                                let body = make_cpim_message_content_body(
                                    &message.message_type,
                                    &message.message_content,
                                    &message.recipient_type,
                                    &message.recipient_uri,
                                    Uuid::new_v4(), // to-do: should be provided by user
                                    &public_user_identity,
                                );

                                let size = body.estimated_size();

                                let mut data = Vec::with_capacity(size);
                                match body.reader() {
                                    Ok(mut reader) => {
                                        match reader.read_to_end(&mut data) {
                                            Ok(_) => {}
                                            Err(_) => {} // to-do: early failure
                                        }
                                    }
                                    Err(_) => {} // to-do: early failure
                                }

                                let from_path_ = from_path.clone();
                                let to_path_ = to_path.clone();

                                let chunk_tx = chunk_tx.clone();

                                demuxer_.start(
                                    from_path_,
                                    to_path_,
                                    content_type,
                                    size,
                                    VectorReader::new(data),
                                    move |status_code, reason_phrase| {
                                        (message.message_result_callback)(
                                            status_code,
                                            reason_phrase,
                                        );
                                    },
                                    chunk_tx,
                                    &rt_,
                                );
                            }
                        }
                    });

                    return Ok(message_tx_);
                }
            }
            _ => {}
        }

        Err(())
    }

    pub fn send_message(&self, message: CPMMessageParam, rt: &Arc<Runtime>) {
        let guard = self.session_state.lock().unwrap();
        match &*guard {
            CPMSessionState::Negotiating(_) => todo!(),
            CPMSessionState::Negotiated(_, _, _, _, _, _, _, _, _, _) => todo!(),
            CPMSessionState::Starting(_, _, _, _, _, _, message_tx) => {
                let message_tx = message_tx.clone();
                rt.spawn(async move {
                    match message_tx.send(message).await {
                        Ok(()) => {}
                        Err(e) => {
                            (e.0.message_result_callback)(500, String::from("Internal Error"))
                        }
                    }
                });
            }
        }
    }
}

struct CPMSessionCPIMMessageWriter {
    message_id: Vec<u8>,
    message_buf: Vec<u8>,
    message_callback: Arc<dyn Fn(&CPIMInfo, &[u8], &[u8]) + Send + Sync>,
}

impl MsrpDataWriter for CPMSessionCPIMMessageWriter {
    fn write(&mut self, data: &[u8]) -> Result<usize, (u16, &'static str)> {
        self.message_buf.extend_from_slice(data);
        Ok(data.len())
    }

    fn complete(&mut self) -> Result<(), (u16, &'static str)> {
        match Body::construct_message(&self.message_buf) {
            Ok(body) => match CPIMMessage::try_from(&body) {
                Ok(cpim_message) => match cpim_message.get_info() {
                    Ok(cpim_message_info) => {
                        if let Some((content_type, content_body, base64_encoded)) =
                            cpim_message.get_message_body()
                        {
                            let content_type: &[u8] = match content_type {
                                Some(content_type) => content_type,
                                None => b"text/plain",
                            };

                            if base64_encoded {
                                if let Ok(decoded_content_body) =
                                    general_purpose::STANDARD.decode(content_body)
                                {
                                    (self.message_callback)(
                                        &cpim_message_info,
                                        content_type,
                                        &decoded_content_body,
                                    );
                                    return Ok(());
                                }
                            } else {
                                (self.message_callback)(
                                    &cpim_message_info,
                                    content_type,
                                    content_body,
                                );
                                return Ok(());
                            }
                        }

                        Err((400, "failed to get content body"))
                    }

                    Err(e) => Err((400, e)),
                },

                Err(e) => Err((400, e)),
            },

            Err(e) => Err((400, e)),
        }
    }
}

struct CPMSessionMessageReceiver {
    callback: Arc<dyn Fn(&CPIMInfo, &[u8], &[u8]) + Send + Sync>,
}

impl MsrpMessageReceiver for CPMSessionMessageReceiver {
    fn on_message(
        &mut self,
        message_id: &[u8],
        content_type: &[u8],
    ) -> Result<Box<dyn MsrpDataWriter + Send + Sync>, (u16, &'static str)> {
        if content_type.eq_ignore_ascii_case(b"message/cpim") {
            // to-do: should check accept-types
            let cb = Arc::clone(&self.callback);
            Ok(Box::new(CPMSessionCPIMMessageWriter {
                message_id: message_id.to_vec(),
                message_buf: Vec::with_capacity(1024),
                message_callback: cb,
            }))
        } else {
            Err((415, "Unsupported Media Type"))
        }
    }
}

pub enum CPMGroupEvent {
    OnInvite,
    OnJoined,
}

pub struct CPMSessionService {
    ongoing_sessions: Arc<Mutex<Vec<(ContactKnownIdentities, Arc<SipSession<CPMSession>>)>>>,
    ongoing_incoming_invitations: Arc<
        Mutex<
            Vec<
                Arc<(
                    ContactKnownIdentities,
                    mpsc::Sender<CPMSessionInvitationResponse>,
                )>,
            >,
        >,
    >,
    ongoing_outgoing_invitations:
        Arc<Mutex<Vec<Arc<(ContactKnownIdentities, mpsc::Sender<CPMMessageParam>)>>>>,

    registered_public_identity: Arc<Mutex<Option<(Arc<SipTransport>, String, String)>>>,

    message_receive_listener:
        Arc<dyn Fn(&Arc<SipSession<CPMSession>>, &[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync>,

    msrp_socket_allocator_function: Arc<
        dyn Fn(
                Option<&MsrpInfo>,
            )
                -> Result<(ClientSocket, String, u16, bool, bool, bool), (u16, &'static str)>
            + Send
            + Sync
            + 'static,
    >,

    msrp_socket_connect_function: Arc<
        dyn Fn(
                ClientSocket,
                &String,
                u16,
                bool,
            )
                -> Pin<Box<dyn Future<Output = Result<ClientStream, (u16, &'static str)>> + Send>>
            + Send
            + Sync
            + 'static,
    >,

    conversation_invite_handler_function:
        Box<dyn Fn(bool, &str, &str, &str, oneshot::Receiver<()>) -> u16 + Send + Sync + 'static>,

    group_invite_handler_function: Box<
        dyn Fn(&str, &str, &str, &str, &str, &str, oneshot::Receiver<()>) -> u16
            + Send
            + Sync
            + 'static,
    >,
    group_invite_event_listener_function: Box<dyn Fn(CPMGroupEvent) + Send + Sync + 'static>,
}

impl CPMSessionService {
    pub fn new<MRL, MAF, MCF, CF, GF, GL>(
        message_receive_listener: MRL,
        msrp_socket_allocator_function: MAF,
        msrp_socket_connect_function: MCF,
        conversation_invite_handler_function: CF,
        group_invite_handler_function: GF,
        group_invite_event_listener_function: GL,
    ) -> CPMSessionService
    where
        MRL: Fn(&Arc<SipSession<CPMSession>>, &[u8], &CPIMInfo, &[u8], &[u8])
            + Send
            + Sync
            + 'static,
        MAF: Fn(
                Option<&MsrpInfo>,
            )
                -> Result<(ClientSocket, String, u16, bool, bool, bool), (u16, &'static str)>
            + Send
            + Sync
            + 'static,
        MCF: Fn(
                ClientSocket,
                &String,
                u16,
                bool,
            )
                -> Pin<Box<dyn Future<Output = Result<ClientStream, (u16, &'static str)>> + Send>>
            + Send
            + Sync
            + 'static,
        CF: Fn(bool, &str, &str, &str, oneshot::Receiver<()>) -> u16 + Send + Sync + 'static,
        GF: Fn(&str, &str, &str, &str, &str, &str, oneshot::Receiver<()>) -> u16
            + Send
            + Sync
            + 'static,
        GL: Fn(CPMGroupEvent) + Send + Sync + 'static,
    {
        CPMSessionService {
            ongoing_sessions: Arc::new(Mutex::new(Vec::new())),
            ongoing_incoming_invitations: Arc::new(Mutex::new(Vec::new())),
            ongoing_outgoing_invitations: Arc::new(Mutex::new(Vec::new())),
            registered_public_identity: Arc::new(Mutex::new(None)),
            message_receive_listener: Arc::new(message_receive_listener),
            msrp_socket_allocator_function: Arc::new(msrp_socket_allocator_function),
            msrp_socket_connect_function: Arc::new(msrp_socket_connect_function),
            conversation_invite_handler_function: Box::new(conversation_invite_handler_function),
            group_invite_handler_function: Box::new(group_invite_handler_function),
            group_invite_event_listener_function: Box::new(group_invite_event_listener_function),
        }
    }

    pub fn set_registered_public_identity(
        &self,
        registered_public_identity: String,
        sip_instance_id: String,
        transport: Arc<SipTransport>,
    ) {
        (*self.registered_public_identity.lock().unwrap()).replace((
            transport,
            registered_public_identity,
            sip_instance_id,
        ));
    }

    pub fn accept_and_open_session(&self, recipient: &str, rt: &Arc<Runtime>) {
        let guard = self.ongoing_incoming_invitations.lock().unwrap();
        for invitation in &*guard {
            let (invitation, tx) = invitation.as_ref();
            if invitation == recipient {
                let tx = tx.clone();
                rt.spawn(async move {
                    match tx.send(CPMSessionInvitationResponse::Accept).await {
                        Ok(()) => {}
                        Err(e) => {}
                    }
                });
                return;
            }
        }
    }

    pub fn send_message<F>(
        &self,
        message_type: &str,
        message_content: &str,
        recipient: &str,
        recipient_type: &RecipientType,
        recipient_uri: &str,
        message_result_callback: F,
        core: &Arc<SipCore>,
        // ctrl_itf: SipTransactionManagerControlInterface,
        rt: &Arc<Runtime>,
    ) where
        F: FnOnce(u16, String) + Send + Sync + 'static,
    {
        let guard = self.ongoing_sessions.lock().unwrap();

        for (known_identities, sip_session) in &*guard {
            if known_identities == recipient {
                sip_session.mark_session_active();

                let cpm_session = sip_session.get_inner();

                let message_result_callback = Arc::new(Mutex::new(Some(message_result_callback)));
                let message_result_callback = move |status_code, reason_phrase| {
                    let mut guard = message_result_callback.lock().unwrap();
                    if let Some(message_result_callback) = guard.take() {
                        message_result_callback(status_code, reason_phrase);
                    }
                };

                let message = CPMMessageParam::new(
                    String::from(message_type),
                    String::from(message_content),
                    recipient,
                    recipient_type,
                    recipient_uri,
                    message_result_callback,
                );

                cpm_session.send_message(message, rt);

                return;
            }
        }

        let guard = self.ongoing_outgoing_invitations.lock().unwrap();

        for invitation in &*guard {
            let (known_identities, message_tx) = invitation.as_ref();
            if known_identities == recipient {
                let message_result_callback = Arc::new(Mutex::new(Some(message_result_callback)));
                let message_result_callback = move |status_code, reason_phrase| {
                    let mut guard = message_result_callback.lock().unwrap();
                    if let Some(message_result_callback) = guard.take() {
                        message_result_callback(status_code, reason_phrase);
                    }
                };

                let message = CPMMessageParam::new(
                    String::from(message_type),
                    String::from(message_content),
                    recipient,
                    recipient_type,
                    recipient_uri,
                    message_result_callback,
                );
                let message_tx = message_tx.clone();
                rt.spawn(async move {
                    match message_tx.send(message).await {
                        Ok(()) => {}
                        Err(e) => {
                            (e.0.message_result_callback)(403, String::from("Forbidden"));
                        }
                    }
                });
                return;
            }
        }

        if let Ok((sock, host, port, tls, active_setup, ipv6)) =
            (self.msrp_socket_allocator_function)(None)
        {
            let path_random = create_raw_alpha_numeric_string(16);
            let path_random = std::str::from_utf8(&path_random).unwrap();
            let path = if tls {
                format!("msrps://{}:{}/{};tcp", &host, port, path_random)
            } else {
                format!("msrp://{}:{}/{};tcp", &host, port, path_random)
            };

            // to-do: accepted types for Group message and others

            let path = path.into_bytes();

            let msrp_info: MsrpInfo = MsrpInfo {
                protocol: if tls { b"TCP/TLS/MSRP" } else { b"TCP/MSRP" },
                address: host.as_bytes(),
                interface_type: if ipv6 {
                    MsrpInterfaceType::IPv6
                } else {
                    MsrpInterfaceType::IPv4
                },
                port,
                path: &path,
                inactive: false,
                direction: MsrpDirection::SendReceive,
                accept_types: if let RecipientType::Chatbot = recipient_type {
                    b"message/cpim"
                } else {
                    b"message/cpim application/im-iscomposing+xm"
                },
                setup_method: if active_setup {
                    MsrpSetupMethod::Active
                } else {
                    MsrpSetupMethod::Passive
                },
                accept_wrapped_types: if let RecipientType::Chatbot = recipient_type {
                    Some(b"multipart/mixed text/plain application/vnd.gsma.rcs-ft-http+xml application/vnd.gsma.rcspushlocation+xml message/imdn+xml application/vnd.gsma.botmessage.v1.0+json application/vnd.gsma.botsuggestion.v1.0+json application/commontemplate+xml")
                } else {
                    Some(b"multipart/mixed text/plain application/vnd.gsma.rcs-ft-http+xml application/vnd.gsma.rcspushlocation+xml message/imdn+xml")
                },
                file_info: None,
            };

            let sdp = String::from(msrp_info);
            let sdp = sdp.into_bytes();
            let sdp = Body::Raw(sdp);
            let sdp = Arc::new(sdp);

            let guard = self.registered_public_identity.lock().unwrap();

            if let Some((transport, contact_identity, instance_id)) = &*guard {
                let mut invite_message = SipMessage::new_request(INVITE, recipient.as_bytes());

                let call_id = String::from(
                    Uuid::new_v4()
                        .as_hyphenated()
                        .encode_lower(&mut Uuid::encode_buffer()),
                );
                invite_message.add_header(Header::new(b"Call-ID", call_id));

                invite_message.add_header(Header::new(b"CSeq", b"1 INVITE"));

                let tag = create_raw_alpha_numeric_string(8);
                let tag = String::from_utf8_lossy(&tag);

                invite_message.add_header(Header::new(
                    b"From",
                    format!("<{}>;tag={}", contact_identity, tag),
                ));

                invite_message.add_header(Header::new(b"To", format!("<{}>", recipient_uri)));

                invite_message.add_header(Header::new(
                    b"Contact",
                    if let RecipientType::Chatbot = recipient_type {
                        format!("<{}>;+sip.instance=\"{}\";+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.session\";+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot\";+g.gsma.rcs.botversion=\"#=1,#=2\"", contact_identity, instance_id)
                    } else {
                        format!("<{}>;+sip.instance=\"{}\";+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.session\"", contact_identity, instance_id)
                    },
                ));

                invite_message.add_header(Header::new(
                    b"Accept-Contact",
                    "*;+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.msg\"",
                ));

                if let RecipientType::Chatbot = recipient_type {
                    invite_message.add_header(Header::new(b"Accept-Contact", "*;+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot\";+g.gsma.rcs.botversion=\"#=1,#=2\";require;explicit"));
                }

                let conversation_id = String::from(
                    Uuid::new_v4()
                        .as_hyphenated()
                        .encode_lower(&mut Uuid::encode_buffer()),
                );
                invite_message.add_header(Header::new(b"Conversation-ID", conversation_id));

                let contribution_id = String::from(
                    Uuid::new_v4()
                        .as_hyphenated()
                        .encode_lower(&mut Uuid::encode_buffer()),
                );
                invite_message.add_header(Header::new(b"Contribution-ID", contribution_id));

                invite_message.add_header(Header::new(
                    b"Allow",
                    b"NOTIFY, OPTIONS, INVITE, UPDATE, CANCEL, BYE, ACK, MESSAGE",
                ));

                invite_message.add_header(Header::new(b"Supported", b"timer"));

                /*
                 * to-do: 3gpp TS 24.229 5.1.3.1
                 *
                 * If the UE supports the Session Timer extension, the UE shall include the option-tag "timer" in the Supported header field and should either insert a Session-Expires header field with the header field value set to the configured session timer interval value, or should not include the Session-Expires header field in the initial INVITE request. The header field value of the Session-Expires header field may be configured using local configuration or using the Session_Timer_Initial_Interval node specified in 3GPP 24.167 [8G]. If the UE is configured with both the local configuration and the Session_Timer_Initial_Interval node specified in 3GPP 24.167 [8G], then the local configuration shall take precedence.
                 * If the UE inserts the Session-Expires header field in the initial INVITE request, the UE may also include the "refresher" parameter with the "refresher" parameter value set to "uac".
                 */
                invite_message.add_header(Header::new(b"Session-Expires", b"1800"));

                invite_message.add_header(Header::new(
                    b"P-Preferred-Service",
                    b"urn:urn-7:3gpp-service.ims.icsi.oma.cpm.session",
                ));

                invite_message.add_header(Header::new(
                    b"P-Preferred-Identity",
                    String::from(contact_identity),
                ));

                invite_message.add_header(Header::new(b"Content-Type", "application/sdp"));

                let content_length = sdp.estimated_size();
                invite_message.add_header(Header::new(
                    b"Content-Length",
                    format!("{}", content_length),
                ));

                let l_sdp_body = Arc::clone(&sdp);
                invite_message.set_body(sdp);

                let req_headers = invite_message.copy_headers();

                let public_user_identity = String::from(contact_identity);

                // let (client_transaction_tx, client_transaction_rx) = mpsc::channel::<mpsc::Sender<CPMMessageParam>>(1);
                let (client_transaction_tx, mut client_transaction_rx) = mpsc::channel::<
                    Result<(SipMessage, CPMSessionInfo, Arc<Body>), (u16, String)>,
                >(1);
                let (message_tx, mut message_rx) = mpsc::channel::<CPMMessageParam>(8);
                let message_receive_listener = Arc::clone(&self.message_receive_listener);
                let message_receive_listener_ = Arc::clone(&message_receive_listener);

                let connect_function = Arc::clone(&self.msrp_socket_connect_function);

                let message_result_callback = Arc::new(Mutex::new(Some(message_result_callback)));
                let message_result_callback = move |status_code, reason_phrase| {
                    let mut guard = message_result_callback.lock().unwrap();
                    if let Some(message_result_callback) = guard.take() {
                        message_result_callback(status_code, reason_phrase);
                    }
                };

                let message = CPMMessageParam::new(
                    String::from(message_type),
                    String::from(message_content),
                    recipient,
                    recipient_type,
                    recipient_uri,
                    message_result_callback,
                );

                let ongoing_dialogs = core.get_ongoing_dialogs();

                let mut known_identities = ContactKnownIdentities::new();
                known_identities.add_identity(String::from(recipient));
                let outgoing_invitation = Arc::new((known_identities, message_tx));
                let outgoing_invitation_ = Arc::clone(&outgoing_invitation);

                let ongoing_outgoing_invitations = Arc::clone(&self.ongoing_outgoing_invitations);
                let ongoing_sessions = Arc::clone(&self.ongoing_sessions);

                let registered_public_identity = Arc::clone(&self.registered_public_identity);

                let tm = core.get_transaction_manager();
                let tm_ = Arc::clone(&tm);

                let rt_ = Arc::clone(rt);

                rt.spawn(async move {
                    // if let Some(message_tx) = client_transaction_rx.recv().await {
                    //     match message_tx.send(message).await {
                    //         Ok(()) => {},
                    //         Err(e) => {
                    //             (e.0.message_result_callback)(403);
                    //         },
                    //     }

                    //     while let Some(message) = message_rx.recv().await {
                    //         match message_tx.send(message).await {
                    //             Ok(()) => {},
                    //             Err(e) => {
                    //                 (e.0.message_result_callback)(403);
                    //             },
                    //         }
                    //     }
                    // }

                    if let Some(res) = client_transaction_rx.recv().await {
                        match res {
                            Ok((resp_message, session_info, r_sdp_body)) => {
                                if let Some(l_sdp) = match l_sdp_body.as_ref() {
                                    Body::Raw(raw) => {
                                        raw.as_sdp()
                                    }
                                    _ => None,
                                } {
                                    if let Some(r_sdp) = match r_sdp_body.as_ref() {
                                        Body::Raw(raw) => {
                                            raw.as_sdp()
                                        }
                                        _ => None,
                                    } {
                                        if let Some(r_msrp_info) = r_sdp.as_msrp_info() {
                                            if let Ok(raddr) = std::str::from_utf8(r_msrp_info.address) {
                                                let raddr = String::from(raddr);
                                                let rport = r_msrp_info.port;
                                                let rpath = r_msrp_info.path.to_vec();

                                                let asserted_contact_uri = session_info.asserted_contact_uri.clone();
                                                let asserted_contact_uri_ = asserted_contact_uri.clone();
                                                let known_identities = session_info.get_contact_known_identities();
                                                let cpm_session = CPMSession::new(session_info);

                                                if let Ok(_) = cpm_session.set_local_sdp(l_sdp_body, sock, tls, host, port, path) {
                                                    if let Ok(_) = cpm_session.set_remote_sdp(r_sdp_body, raddr, rport, rpath) {
                                                        let cpm_session = Arc::new(cpm_session);
                                                        let cpm_session_ = Arc::clone(&cpm_session);

                                                        let (d_tx, mut d_rx) = mpsc::channel(1);

                                                        let ongoing_dialogs_ = Arc::clone(&ongoing_dialogs);

                                                        rt_.spawn(async move {
                                                            if let Some(dialog) = d_rx.recv().await {
                                                                ongoing_dialogs_.remove_dialog(&dialog);
                                                            }
                                                        });

                                                        if let Ok(dialog) = SipDialog::try_new_as_uac(&req_headers, &resp_message, move |d| {
                                                            match d_tx.blocking_send(d) {
                                                                Ok(()) => {},
                                                                Err(e) => {},
                                                            }
                                                        }) {
                                                            let dialog = Arc::new(dialog);

                                                            ongoing_dialogs.add_dialog(&dialog);

                                                            let (sess_tx, mut sess_rx) = mpsc::channel::<SipSessionEvent>(8);

                                                            let sip_session = SipSession::new(&cpm_session, SipSessionEventReceiver {
                                                                tx: sess_tx,
                                                                rt: Arc::clone(&rt_),
                                                            });

                                                            let sip_session = Arc::new(sip_session);
                                                            let sip_session_ = Arc::clone(&sip_session);

                                                            let public_user_identity_ = public_user_identity.clone();

                                                            sip_session.setup_confirmed_dialog(&dialog, CPMSessionDialogEventReceiver {
                                                                public_user_identity: public_user_identity_,
                                                                sip_session: Arc::clone(&sip_session),
                                                                ongoing_sessions: Arc::clone(&ongoing_sessions),
                                                                message_receive_listener: Arc::new(move |cpim_info, content_type, content_body| {
                                                                    message_receive_listener_(&sip_session_, asserted_contact_uri_.as_bytes(), cpim_info, content_type, content_body)
                                                                }),
                                                                msrp_socket_connect_function: Arc::clone(
                                                                    &connect_function,
                                                                ),
                                                                rt: Arc::clone(&rt_),
                                                            });

                                                            if let Some((timeout, refresher)) =
                                                            choose_timeout_on_client_transaction_completion(None, &resp_message)
                                                            {
                                                                match refresher {
                                                                    Refresher::UAC => sip_session.schedule_refresh(timeout, true, &rt_),

                                                                    Refresher::UAS => sip_session.schedule_refresh(timeout, false, &rt_),
                                                                }
                                                            }

                                                            let sip_session_1 = Arc::clone(&sip_session);
                                                            let sip_session_2 = Arc::clone(&sip_session);

                                                            let registered_public_identity_ = Arc::clone(&registered_public_identity);

                                                            let tm = Arc::clone(&tm_);
                                                            let rt = Arc::clone(&rt_);

                                                            rt_.spawn(async move {
                                                                while let Some(ev) = sess_rx.recv().await {
                                                                    match ev {
                                                                        SipSessionEvent::ShouldRefresh(dialog) => {
                                                                            if let Ok(mut message) =
                                                                                dialog.make_request(b"UPDATE", None)
                                                                            {
                                                                                message.add_header(Header::new(
                                                                                    b"Supported",
                                                                                    b"timer",
                                                                                ));

                                                                                let guard =
                                                                                registered_public_identity_.lock().unwrap();

                                                                                if let Some((transport, _, _)) = &*guard {
                                                                                    tm.send_request(
                                                                                        message,
                                                                                        transport,
                                                                                        UpdateMessageCallbacks {
                                                                                            // session_expires: None,
                                                                                            dialog,
                                                                                            sip_session: Arc::clone(&sip_session_1),
                                                                                            rt: Arc::clone(&rt),
                                                                                        },
                                                                                        &rt,
                                                                                    );
                                                                                }
                                                                            }
                                                                        }

                                                                        SipSessionEvent::Expired(dialog) => {
                                                                            if let Ok(message) = dialog.make_request(b"BYE", None) {
                                                                                let guard =
                                                                                registered_public_identity_.lock().unwrap();

                                                                                if let Some((transport, _, _)) = &*guard {
                                                                                    tm.send_request(
                                                                                        message,
                                                                                        transport,
                                                                                        ClientTransactionNilCallbacks {},
                                                                                        &rt,
                                                                                    );
                                                                                }
                                                                            }
                                                                        }

                                                                        _ => {}
                                                                    }
                                                                }
                                                            });

                                                            if let Ok(ack_message) = dialog.make_request(b"ACK", Some(1)) {

                                                                let rt = Arc::clone(&rt_);

                                                                if let Some(transport) = {
                                                                    let i;
                                                                    {
                                                                        let guard = registered_public_identity.lock().unwrap();

                                                                        if let Some((transport, _, _)) = &*guard {
                                                                            i = Some(Arc::clone(transport))
                                                                        } else {
                                                                            i = None
                                                                        }
                                                                    }
                                                                    i
                                                                } {
                                                                    tm_.send_request(
                                                                        ack_message,
                                                                        &transport,
                                                                        ClientTransactionNilCallbacks {},
                                                                        &rt_,
                                                                    );

                                                                    let ongoing_sessions_ = Arc::clone(&ongoing_sessions);

                                                                    if let Ok(message_tx) = cpm_session.start(&public_user_identity, move |cpim_info, content_type, mesage_buf| {
                                                                        message_receive_listener(&sip_session_2, asserted_contact_uri.as_bytes(), cpim_info, content_type, mesage_buf)
                                                                    }, &connect_function, move || {
                                                                        let mut guard = ongoing_sessions_.lock().unwrap();
                                                                        if let Some(idx) = guard.iter().position(|(_, sip_session)| {
                                                                            let inner = sip_session.get_inner();
                                                                            Arc::ptr_eq(&inner, &cpm_session_)
                                                                        }) {
                                                                            guard.swap_remove(idx);
                                                                        }
                                                                    }, &rt) {

                                                                        match message_tx.send(message).await {
                                                                            Ok(()) => {},
                                                                            Err(e) => {
                                                                                (e.0.message_result_callback)(403, String::from("Forbidden"));
                                                                            },
                                                                        }

                                                                        while let Some(message) = message_rx.recv().await {
                                                                            match message_tx.send(message).await {
                                                                                Ok(()) => {},
                                                                                Err(e) => {
                                                                                    (e.0.message_result_callback)(403, String::from("Forbidden"));
                                                                                },
                                                                            }
                                                                        }

                                                                        let mut guard = ongoing_sessions.lock().unwrap();

                                                                        guard.push((known_identities, sip_session));

                                                                        let mut guard = ongoing_outgoing_invitations.lock().unwrap();

                                                                        if let Some(idx) = guard.iter().position(|outgoing_invitation| Arc::ptr_eq(outgoing_invitation, &outgoing_invitation_)) {
                                                                            guard.swap_remove(idx);
                                                                        }

                                                                        return;
                                                                    }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            Err((error_code, error_reason)) => {
                                (message.message_result_callback)(error_code, error_reason);
                                return;
                            }
                        }
                    }

                    (message.message_result_callback)(500, String::from("Internal Error"));
                });

                tm.send_request(
                    invite_message,
                    transport,
                    CPMSessionInviteCallbacks {
                        // l_sdp,
                        recipient_type: RecipientType::from(recipient_type),
                        client_transaction_tx,
                        // message_result_callback: Box::new(message_result_callback),
                        // message_receive_listener: Arc::clone(&self.message_receive_listener),
                        rt: Arc::clone(rt),
                    },
                    rt,
                );

                self.ongoing_outgoing_invitations
                    .lock()
                    .unwrap()
                    .push(outgoing_invitation);
                return;
            }
        }

        (message_result_callback)(500, String::from("Internal Error"));
    }
}

struct CPMSessionInviteCallbacks {
    // l_sdp: Arc<Body>,
    recipient_type: RecipientType,
    client_transaction_tx:
        mpsc::Sender<Result<(SipMessage, CPMSessionInfo, Arc<Body>), (u16, String)>>,
    // client_transaction_tx: mpsc::Sender<mpsc::Sender<CPMMessageParam>>,
    // message_result_callback: Box<dyn Fn(u16) + Send + Sync>,
    // message_receive_listener: Arc<dyn Fn(&[u8], &[u8], &[u8]) + Send + Sync>,
    rt: Arc<Runtime>,
}

impl ClientTransactionCallbacks for CPMSessionInviteCallbacks {
    fn on_provisional_response(&self, message: SipMessage) {}

    fn on_final_response(&self, message: SipMessage) {
        if let SipMessage::Response(l, _, _) = &message {
            let mut status_code = 500;
            let mut reason_phrase = String::from("Internal Error");
            if l.status_code >= 200 && l.status_code < 300 {
                if let Some(session_info) = get_cpm_session_info_from_message(&message, true) {
                    loop {
                        let accepting_chatbot_session_is_forbidden =
                            if let RecipientType::Chatbot = self.recipient_type {
                                match session_info.cpm_contact.service_type {
                                    CPMServiceType::Chatbot => false,
                                    _ => true,
                                }
                            } else {
                                false
                            };

                        if accepting_chatbot_session_is_forbidden {
                            status_code = 606;
                            reason_phrase = String::from("Not Acceptable");
                            break;
                        }

                        if let Some(resp_body) = message.get_body() {
                            // let mut sdp: Option<Sdp> = None;

                            if let Some(headers) = message.headers() {
                                if let Some(header) = header::search(headers, b"Content-Type", true)
                                {
                                    if let Some(content_type) =
                                        header.get_value().as_header_field().as_content_type()
                                    {
                                        if content_type
                                            .major_type
                                            .eq_ignore_ascii_case(b"application")
                                            && content_type.sub_type.eq_ignore_ascii_case(b"sdp")
                                        {
                                            // match resp_body.as_ref() {
                                            //     Body::Raw(raw) => {
                                            //         sdp = raw.as_sdp();
                                            //     }
                                            //     _ => {}
                                            // }
                                            let client_transaction_tx =
                                                self.client_transaction_tx.clone();
                                            let sdp_body = Arc::clone(&resp_body);
                                            self.rt.spawn(async move {
                                                match client_transaction_tx
                                                    .send(Ok((message, session_info, sdp_body)))
                                                    .await
                                                {
                                                    Ok(()) => {}
                                                    Err(e) => {}
                                                }
                                            });

                                            return;
                                        }
                                    }
                                }
                            }

                            // let l_sdp_body = Arc::clone(&self.l_sdp);
                            // let r_sdp_body = resp_body;

                            // if let Some(sdp) = sdp {
                            //     if let Some(msrp_info) = sdp.as_msrp_info() {
                            //         if let Some(l_sdp) = match l_sdp_body.as_ref() {
                            //             Body::Raw(raw) => {
                            //                 raw.as_sdp()
                            //             }
                            //             _ => None,
                            //         } {
                            //             if let Some(l_msrp_info) = l_sdp.as_msrp_info() {
                            //                 let message_receive_listener = Arc::clone(&self.message_receive_listener);
                            //                 let cpm_session = CPMSession::new(session_info, move |content_type, mesage_buf| {
                            //                     message_receive_listener("", content_type, mesage_buf)
                            //                 });

                            //                 if let Ok(_) = cpm_session.set_local_sdp(l_sdp_body, sock, tls, laddr, lport, lpath) {
                            //                     if let Ok(_) = cpm_session.set_remote_sdp(r_sdp_body, raddr, rport, rpath) {

                            //                         if let Ok(message_tx) = cpm_session.start(connect_function, session_end_function, &self.rt) {
                            //                             let client_transaction_tx = self.client_transaction_tx.clone();

                            //                             self.rt.spawn(async move {
                            //                                 client_transaction_tx.send(message_tx).await;
                            //                             });

                            //                             return;
                            //                         }
                            //                     }
                            //                 }
                            //             }
                            //         }
                            //     }
                            // }
                        }
                        break;
                    }
                }
            } else {
                status_code = l.status_code;
                reason_phrase = String::from_utf8_lossy(&l.reason_phrase).to_string();
            }

            let client_transaction_tx = self.client_transaction_tx.clone();
            self.rt.spawn(async move {
                match client_transaction_tx
                    .send(Err((status_code, reason_phrase)))
                    .await
                {
                    Ok(()) => {}
                    Err(e) => {}
                }
            });
        }
    }

    fn on_transport_error(&self) {
        todo!()
    }
}

pub(crate) struct UpdateMessageCallbacks<T> {
    // session_expires: Option<Header>,
    pub(crate) dialog: Arc<SipDialog>,
    pub(crate) sip_session: Arc<SipSession<T>>,
    pub(crate) rt: Arc<Runtime>,
}

impl<T> ClientTransactionCallbacks for UpdateMessageCallbacks<T> {
    fn on_provisional_response(&self, message: SipMessage) {}

    fn on_final_response(&self, message: SipMessage) {
        if let SipMessage::Response(l, _, _) = &message {
            if l.status_code >= 200 && l.status_code < 300 {
                if let Some((timeout, refresher)) =
                    choose_timeout_on_client_transaction_completion(None, &message)
                {
                    match refresher {
                        Refresher::UAC => {
                            self.sip_session.schedule_refresh(timeout, true, &self.rt)
                        }

                        Refresher::UAS => {
                            self.sip_session.schedule_refresh(timeout, false, &self.rt)
                        }
                    }
                }
            } else {
                if l.status_code == 404
                    || l.status_code == 410
                    || l.status_code == 416
                    || (l.status_code >= 482 && l.status_code <= 485)
                    || l.status_code == 489
                    || l.status_code == 604
                {
                    self.dialog.on_terminating_response(&message, &self.rt);
                }
            }
        }
    }

    fn on_transport_error(&self) {}
}

pub(crate) struct CPMSessionDialogEventReceiver {
    pub(crate) sip_session: Arc<SipSession<CPMSession>>,
    pub(crate) ongoing_sessions:
        Arc<Mutex<Vec<(ContactKnownIdentities, Arc<SipSession<CPMSession>>)>>>,
    pub(crate) public_user_identity: String,
    pub(crate) message_receive_listener: Arc<dyn Fn(&CPIMInfo, &[u8], &[u8]) + Send + Sync>,
    pub(crate) msrp_socket_connect_function: Arc<
        dyn Fn(
                ClientSocket,
                &String,
                u16,
                bool,
            )
                -> Pin<Box<dyn Future<Output = Result<ClientStream, (u16, &'static str)>> + Send>>
            + Send
            + Sync,
    >,
    pub(crate) rt: Arc<Runtime>,
}

impl SipDialogEventCallbacks for CPMSessionDialogEventReceiver {
    fn on_ack(&self, _transaction: &Arc<ServerTransaction>) {
        let ongoing_sessions = Arc::clone(&self.ongoing_sessions);
        let sip_session_ = Arc::clone(&self.sip_session);
        let session = self.sip_session.get_inner();
        let message_receive_listener = Arc::clone(&self.message_receive_listener);
        match session.start(
            &self.public_user_identity,
            move |cpim_info, content_type, content_body| {
                message_receive_listener(cpim_info, content_type, content_body)
            },
            &self.msrp_socket_connect_function,
            move || {
                let mut guard = ongoing_sessions.lock().unwrap();
                if let Some(idx) = guard
                    .iter()
                    .position(|(_, sip_session)| Arc::ptr_eq(sip_session, &sip_session_))
                {
                    guard.swap_remove(idx);
                }
            },
            &self.rt,
        ) {
            Ok(_) => {}
            Err(()) => {}
        };
    }

    fn on_new_request(
        &self,
        transaction: Arc<ServerTransaction>,
        tx: mpsc::Sender<ServerTransactionEvent>,
        // timer: &Timer,
        rt: &Arc<Runtime>,
    ) -> Option<(u16, bool)> {
        let message = transaction.message();

        if let SipMessage::Request(l, req_headers, req_body) = message {
            if l.method == UPDATE {
                if let Some(headers) = req_headers {
                    if let Some(h) = search(headers, b"Supported", true) {
                        if h.supports(b"timer") {
                            if let Ok(Some((timeout, refresher))) =
                                choose_timeout_for_server_transaction_response(
                                    &transaction,
                                    true,
                                    Refresher::UAC,
                                )
                            {
                                // to-do: not always UAC ?
                                if let Some(mut resp_message) = server_transaction::make_response(
                                    message,
                                    transaction.to_tag(),
                                    200,
                                    b"Ok",
                                ) {
                                    match refresher {
                                        Refresher::UAC => {
                                            resp_message.add_header(Header::new(
                                                b"Session-Expires",
                                                format!("{};refresher=uac", timeout),
                                            ));

                                            self.sip_session.schedule_refresh(timeout, false, rt);
                                        }
                                        Refresher::UAS => {
                                            resp_message.add_header(Header::new(
                                                b"Session-Expires",
                                                format!("{};refresher=uas", timeout),
                                            ));

                                            self.sip_session.schedule_refresh(timeout, true, rt);
                                        }
                                    }

                                    server_transaction::send_response(
                                        transaction,
                                        resp_message,
                                        tx,
                                        rt,
                                    );
                                }

                                return Some((200, false));
                            }
                        }
                    }

                    if let Some(resp_message) =
                        server_transaction::make_response(message, transaction.to_tag(), 200, b"Ok")
                    {
                        server_transaction::send_response(transaction, resp_message, tx, rt);
                    }
                }

                return Some((200, false));
            }
        }

        None
    }

    fn on_terminating_request(&self, _message: &SipMessage) {
        let mut guard = self.ongoing_sessions.lock().unwrap();
        if let Some(idx) = guard
            .iter()
            .position(|(_, sip_session)| Arc::ptr_eq(&sip_session, &self.sip_session))
        {
            guard.swap_remove(idx);
        }
    }

    fn on_terminating_response(&self, message: &SipMessage) {}
}

pub struct CPMSessionServiceWrapper {
    pub service: Arc<CPMSessionService>,
    pub tm: Arc<SipTransactionManager>,
}

impl TransactionHandler for CPMSessionServiceWrapper {
    fn handle_transaction(
        &self,
        transaction: &Arc<ServerTransaction>,
        ongoing_dialogs: &Arc<Mutex<Vec<Arc<SipDialog>>>>,
        channels: &mut Option<(
            mpsc::Sender<ServerTransactionEvent>,
            mpsc::Receiver<ServerTransactionEvent>,
        )>,
        rt: &Arc<Runtime>,
    ) -> bool {
        let message = transaction.message();
        if let SipMessage::Request(req_line, Some(req_headers), Some(req_body)) = message {
            if req_line.method == INVITE {
                let mut is_cpm_session_invitation = false;

                for header in req_headers {
                    if header.get_name().eq_ignore_ascii_case(b"Accept-Contact") {
                        if header.get_value().as_header_field().contains_icsi_ref(
                            b"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.session",
                        ) {
                            is_cpm_session_invitation = true;
                            break;
                        }
                    }
                }

                if is_cpm_session_invitation {
                    let mut sdp = None;

                    for header in req_headers {
                        if header.get_name().eq_ignore_ascii_case(b"Content-Type") {
                            if let Some(content_type) =
                                header.get_value().as_header_field().as_content_type()
                            {
                                if content_type.major_type.eq_ignore_ascii_case(b"application")
                                    && content_type.sub_type.eq_ignore_ascii_case(b"sdp")
                                {
                                    match req_body.as_ref() {
                                        Body::Raw(raw) => {
                                            sdp = raw.as_sdp();
                                        }
                                        _ => {}
                                    }
                                }
                            }
                        }
                    }

                    if let Some(sdp) = sdp {
                        if let Some(msrp_info) = sdp.as_msrp_info() {
                            if let Some(session_info) =
                                get_cpm_session_info_from_message(message, false)
                            {
                                let auto_accept_response_code: u16;

                                let (cancel_tx, cancel_rx) = oneshot::channel::<()>();

                                match session_info.cpm_contact.service_type {
                                    CPMServiceType::Group => loop {
                                        if let (
                                            Some(conversation_id),
                                            Some(contribution_id),
                                            Some(subject),
                                            Some(referred_by),
                                        ) = (
                                            &session_info.conversation_id,
                                            &session_info.contribution_id,
                                            &session_info.subject,
                                            &session_info.referred_by,
                                        ) {
                                            if let Ok(subject) = urlencoding::decode(subject) {
                                                if let Some(referred_by) = referred_by
                                                    .as_bytes()
                                                    .as_name_addresses()
                                                    .first()
                                                {
                                                    if let Some(referred_by_name) =
                                                        referred_by.display_name
                                                    {
                                                        if let Ok(referred_by_name) =
                                                            std::str::from_utf8(referred_by_name)
                                                        {
                                                            if let Some(uri_part) =
                                                                &referred_by.uri_part
                                                            {
                                                                if let Ok(referred_by_uri) =
                                                                    std::str::from_utf8(
                                                                        uri_part.uri,
                                                                    )
                                                                {
                                                                    match msrp_info.direction {
                                                                            MsrpDirection::SendReceive => {
                                                                                auto_accept_response_code = (self.service.group_invite_handler_function)(&session_info.asserted_contact_uri, conversation_id, contribution_id, &subject, referred_by_name, referred_by_uri, cancel_rx);
                                                                                break;
                                                                            }

                                                                            _ => {
                                                                                (self.service.group_invite_event_listener_function)(CPMGroupEvent::OnInvite);
                                                                                return false;
                                                                            }
                                                                        }
                                                                }
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }

                                        return false;
                                    },

                                    _ => {
                                        let mut is_chatbot_session = false;

                                        for header in req_headers {
                                            if header
                                                .get_name()
                                                .eq_ignore_ascii_case(b"Accept-Contact")
                                            {
                                                if header.get_value().as_header_field().contains_iari_ref(b"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot") {
                                                    is_chatbot_session = true;
                                                    break;
                                                }
                                            }
                                        }

                                        if is_chatbot_session {
                                            match session_info.cpm_contact.service_type {
                                                CPMServiceType::Chatbot => {}
                                                _ => {
                                                    if let Some(resp_message) =
                                                        server_transaction::make_response(
                                                            message,
                                                            transaction.to_tag(),
                                                            606,
                                                            b"Not Acceptable",
                                                        )
                                                    {
                                                        if let Some((tx, _)) = channels.take() {
                                                            server_transaction::send_response(
                                                                Arc::clone(transaction),
                                                                resp_message,
                                                                tx,
                                                                // &timer,
                                                                rt,
                                                            );
                                                        }
                                                    }

                                                    return true;
                                                }
                                            }

                                            // to-do: check privacy settings and return 606 if appropriate
                                        }

                                        loop {
                                            if let (Some(conversation_id), Some(contribution_id)) = (
                                                &session_info.conversation_id,
                                                &session_info.contribution_id,
                                            ) {
                                                match msrp_info.direction {
                                                    MsrpDirection::SendOnly => {
                                                        if session_info.is_deferred_session {
                                                            auto_accept_response_code = (self.service.conversation_invite_handler_function)(true, &session_info.asserted_contact_uri, contribution_id, conversation_id, cancel_rx);
                                                            break;
                                                        }
                                                    }

                                                    MsrpDirection::SendReceive => {
                                                        auto_accept_response_code = (self
                                                            .service
                                                            .conversation_invite_handler_function)(
                                                            false,
                                                            &session_info.asserted_contact_uri,
                                                            &contribution_id,
                                                            &conversation_id,
                                                            cancel_rx,
                                                        );
                                                        break;
                                                    }

                                                    _ => {}
                                                }
                                            }

                                            return false;
                                        }
                                    }
                                }

                                if auto_accept_response_code >= 200
                                    && auto_accept_response_code < 300
                                {
                                    if let Some((tx, rx)) = channels.take() {
                                        try_accept_invitation(
                                            session_info,
                                            tx,
                                            rx,
                                            None,
                                            req_body,
                                            msrp_info,
                                            &self.service.msrp_socket_allocator_function,
                                            &self.service.msrp_socket_connect_function,
                                            &self.service.message_receive_listener,
                                            transaction,
                                            message,
                                            &self.service.registered_public_identity,
                                            ongoing_dialogs,
                                            &self.service.ongoing_sessions,
                                            &self.tm,
                                            rt,
                                        )
                                    } else {
                                        // 500
                                    }

                                    return true;
                                } else if auto_accept_response_code >= 100
                                    && auto_accept_response_code < 200
                                {
                                    if let Some((tx, mut rx)) = channels.take() {
                                        if let Some(mut resp_message) =
                                            server_transaction::make_response(
                                                message,
                                                transaction.to_tag(),
                                                180,
                                                b"Ringing",
                                            )
                                        {
                                            resp_message.add_header(Header::new(b"Allow", b"NOTIFY, OPTIONS, INVITE, UPDATE, CANCEL, BYE, ACK, MESSAGE"));

                                            {
                                                let guard = self
                                                    .service
                                                    .registered_public_identity
                                                    .lock()
                                                    .unwrap();

                                                if let Some((
                                                    transport,
                                                    contact_identity,
                                                    instance_id,
                                                )) = &*guard
                                                {
                                                    let transport_ = transaction.transport();
                                                    if Arc::ptr_eq(transport, transport_) {
                                                        resp_message.add_header(Header::new(
                                                            b"Contact",
                                                            format!(
                                                                "<{}>;+sip.instance=\"{}\"",
                                                                contact_identity, instance_id
                                                            ),
                                                        ));
                                                    } // to-do: treat as error if transport has changed somehow
                                                }
                                            }

                                            let (d_tx, mut d_rx) = mpsc::channel(1);

                                            let ongoing_dialogs_ = Arc::clone(ongoing_dialogs);

                                            rt.spawn(async move {
                                                if let Some(dialog) = d_rx.recv().await {
                                                    ongoing_dialogs_.remove_dialog(&dialog);
                                                }
                                            });

                                            if let Ok(dialog) = SipDialog::try_new_as_uas(
                                                message,
                                                &resp_message,
                                                move |d| match d_tx.blocking_send(d) {
                                                    Ok(()) => {}
                                                    Err(e) => {}
                                                },
                                            ) {
                                                let dialog = Arc::new(dialog);

                                                ongoing_dialogs.add_dialog(&dialog);

                                                // dialog.register_transaction(Arc::clone(transaction));

                                                let (tx_, rx_) =
                                                    mpsc::channel::<ServerTransactionEvent>(8);

                                                let known_identities =
                                                    session_info.get_contact_known_identities();

                                                let invitation = CPMSessionInvitation::new(
                                                    session_info,
                                                    Arc::clone(&req_body),
                                                    Arc::clone(&transaction),
                                                    tx.clone(),
                                                    rx_,
                                                    dialog,
                                                );

                                                let (iv_tx, mut iv_rx) =
                                                    mpsc::channel::<CPMSessionInvitationResponse>(
                                                        1,
                                                    );
                                                let iv_tx_ = iv_tx.clone();
                                                let iv = Arc::new((known_identities, iv_tx_));
                                                let iv_ = Arc::clone(&iv);

                                                let message_receive_listener = Arc::clone(
                                                    &self.service.message_receive_listener,
                                                );
                                                let msrp_socket_allocator_function = Arc::clone(
                                                    &self.service.msrp_socket_allocator_function,
                                                );
                                                let msrp_socket_connect_function = Arc::clone(
                                                    &self.service.msrp_socket_connect_function,
                                                );
                                                let registered_public_identity = Arc::clone(
                                                    &self.service.registered_public_identity,
                                                );
                                                let ongoing_dialogs = Arc::clone(ongoing_dialogs);
                                                let ongoing_sessions =
                                                    Arc::clone(&self.service.ongoing_sessions);

                                                let ongoing_incoming_invitations_ = Arc::clone(
                                                    &self.service.ongoing_incoming_invitations,
                                                );

                                                let tm_ = Arc::clone(&self.tm);
                                                let rt_ = Arc::clone(rt);

                                                rt.spawn(async move {
                                                    match iv_rx.recv().await {
                                                        Some(
                                                            CPMSessionInvitationResponse::Accept,
                                                        ) => {
                                                            try_accept_hanging_invitation(
                                                                invitation,
                                                                &msrp_socket_allocator_function,
                                                                &msrp_socket_connect_function,
                                                                &message_receive_listener,
                                                                &registered_public_identity,
                                                                &ongoing_dialogs,
                                                                &ongoing_sessions,
                                                                &tm_,
                                                                &rt_,
                                                            );
                                                        }
                                                        Some(
                                                            CPMSessionInvitationResponse::Dispose,
                                                        ) => match cancel_tx.send(()) {
                                                            Ok(()) => {}
                                                            Err(()) => {}
                                                        },
                                                        _ => {}
                                                    }

                                                    let mut guard = ongoing_incoming_invitations_
                                                        .lock()
                                                        .unwrap();
                                                    if let Some(idx) = guard
                                                        .iter()
                                                        .position(|iv| Arc::ptr_eq(iv, &iv_))
                                                    {
                                                        guard.swap_remove(idx);
                                                    }
                                                });

                                                {
                                                    self.service
                                                        .ongoing_incoming_invitations
                                                        .lock()
                                                        .unwrap()
                                                        .push(iv);
                                                }

                                                rt.spawn(async move {
                                                    while let Some(ev) = rx.recv().await {
                                                        match ev {
                                                            ServerTransactionEvent::Cancelled => {
                                                                match iv_tx.send(CPMSessionInvitationResponse::Dispose).await {
                                                                    Ok(()) => {},
                                                                    Err(e) => {},
                                                                }
                                                            }
                                                            _ => {}
                                                        }

                                                        match tx_.send(ev).await {
                                                            Ok(()) => {}
                                                            Err(e) => {}
                                                        }
                                                    }
                                                });

                                                server_transaction::send_response(
                                                    Arc::clone(transaction),
                                                    resp_message,
                                                    tx,
                                                    // &timer,
                                                    rt,
                                                );

                                                return true;
                                            }
                                        }
                                    }

                                    return true;
                                }
                            }

                            return false;
                        }
                    }

                    if let Some(resp_message) = server_transaction::make_response(
                        message,
                        transaction.to_tag(),
                        400,
                        b"Bad Request",
                    ) {
                        if let Some((tx, _)) = channels.take() {
                            server_transaction::send_response(
                                Arc::clone(transaction),
                                resp_message,
                                tx,
                                // &timer,
                                rt,
                            );
                        }
                    }

                    return true;
                }
            }
        }

        false
    }
}
