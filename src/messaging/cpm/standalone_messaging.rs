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

extern crate base64;

use std::io::Read;
use std::pin::Pin;
use std::sync::{Arc, Mutex};

use base64::{engine::general_purpose, Engine as _};
use futures::Future;
use rust_rcs_core::ffi::log::platform_log;
use rust_rcs_core::internet::body::multipart_body::MultipartBody;
use rust_rcs_core::internet::body::VectorReader;
use rust_rcs_core::io::network::stream::{ClientSocket, ClientStream};
use rust_rcs_core::msrp::info::msrp_info_reader::AsMsrpInfo;
use rust_rcs_core::msrp::info::{MsrpDirection, MsrpInfo, MsrpInterfaceType, MsrpSetupMethod};
use rust_rcs_core::msrp::msrp_channel::MsrpChannel;
use rust_rcs_core::msrp::msrp_demuxer::MsrpDemuxer;
use rust_rcs_core::msrp::msrp_muxer::{MsrpDataWriter, MsrpMessageReceiver, MsrpMuxer};
use rust_rcs_core::msrp::msrp_transport::msrp_transport_start;
use rust_rcs_core::sip::sip_core::SipDialogCache;
use rust_rcs_core::sip::sip_session::{
    choose_timeout_for_server_transaction_response, Refresher, SipSession, SipSessionEvent,
    SipSessionEventReceiver,
};
use rust_rcs_core::sip::sip_transaction::client_transaction::ClientTransactionNilCallbacks;

use rust_strict_sdp::{AsSDP, Sdp};

use tokio::runtime::Runtime;
use tokio::sync::mpsc;

use uuid::Uuid;

use rust_rcs_core::cpim::{CPIMInfo, CPIMMessage};

use rust_rcs_core::internet::body::message_body::MessageBody;
use rust_rcs_core::internet::header::{search, HeaderSearch};
use rust_rcs_core::internet::header_field::AsHeaderField;
use rust_rcs_core::internet::Body;
use rust_rcs_core::internet::{header, Header};

use rust_rcs_core::internet::headers::{AsContentType, Supported};

use rust_rcs_core::internet::name_addr::AsNameAddr;

use rust_rcs_core::io::{DynamicChain, Serializable};
use rust_rcs_core::sip::sip_transaction::server_transaction;
use rust_rcs_core::sip::SipDialog;
use rust_rcs_core::sip::{ClientTransactionCallbacks, SipDialogEventCallbacks};
use rust_rcs_core::sip::{ServerTransaction, UPDATE};
use rust_rcs_core::sip::{ServerTransactionEvent, SipTransactionManager};
use rust_rcs_core::sip::{SipCore, SipTransport};

use rust_rcs_core::sip::SipMessage;
use rust_rcs_core::sip::TransactionHandler;

use rust_rcs_core::sip::INVITE;
use rust_rcs_core::sip::MESSAGE;

use rust_rcs_core::util::rand::{self, create_raw_alpha_numeric_string};
use rust_rcs_core::util::raw_string::{StrEq, StrFind};
// use rust_rcs_core::util::timer::Timer;

use crate::chat_bot::cpim::GetBotInfo;
use crate::messaging::ffi::RecipientType;

use super::session::{get_cpm_session_info_from_message, CPMSessionInfo, UpdateMessageCallbacks};
use super::sip::cpm_accept_contact::CPMAcceptContact;
use super::sip::cpm_contact::{AsCPMContact, CPMServiceType};
use super::{make_cpim_message_content_body, CPMMessageParam};

const LOG_TAG: &str = "messaging";

struct StandaloneLargeModeSendSession {
    state: Arc<
        Mutex<(
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
            Option<mpsc::Sender<Option<Vec<u8>>>>,
        )>,
    >,

    message_body: Arc<Body>,
}

impl StandaloneLargeModeSendSession {
    pub fn new(
        l_sdp: Arc<Body>,
        r_sdp: Arc<Body>,
        sock: ClientSocket,
        tls: bool,
        laddr: String,
        lport: u16,
        lpath: Vec<u8>,
        raddr: String,
        rport: u16,
        rpath: Vec<u8>,
        message_body: Arc<Body>,
    ) -> StandaloneLargeModeSendSession {
        StandaloneLargeModeSendSession {
            state: Arc::new(Mutex::new((
                l_sdp,
                r_sdp,
                Some(sock),
                tls,
                laddr,
                lport,
                lpath,
                raddr,
                rport,
                rpath,
                None,
            ))),
            message_body,
        }
    }

    pub fn start(
        &self,
        message_result_callback: &Arc<dyn Fn(u16, String) + Send + Sync + 'static>,
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
        rt: &Arc<Runtime>,
    ) {
        let mut guard = self.state.lock().unwrap();
        if let Some(sock) = guard.2.take() {
            let addr = String::from(&guard.7);
            let port = guard.8;
            let tls = guard.3;

            let lpath = guard.6.clone();
            let rpath = guard.9.clone();

            let (data_tx, data_rx) = mpsc::channel(8);
            let data_tx_ = data_tx.clone();

            let message_result_callback = Arc::clone(message_result_callback);
            let message_result_callback_ = Arc::clone(&message_result_callback);

            let connect_function = Arc::clone(&connect_function);

            let message_body = Arc::clone(&self.message_body);

            let rt_ = Arc::clone(rt);

            rt.spawn(async move {
                if let Ok(cs) = connect_function(sock, &addr, port, tls).await {
                    let t_id = format!("");

                    let from_path = lpath.clone();
                    let to_path = rpath.clone();

                    let demuxer = MsrpDemuxer::new(from_path, to_path);
                    let demuxer = Arc::new(demuxer);
                    let demuxer_ = Arc::clone(&demuxer);

                    let muxer = MsrpMuxer::new(StandaloneLargeModeNilReceiver {});

                    let rt = Arc::clone(&rt_);

                    let from_path = lpath.clone();
                    let to_path = rpath.clone();

                    let data_tx_ = data_tx.clone();

                    let mut session_channel = MsrpChannel::new(from_path, to_path, demuxer_, muxer);

                    msrp_transport_start(
                        cs,
                        t_id,
                        data_tx_,
                        data_rx,
                        move |message_chunk| session_channel.on_message(message_chunk),
                        move || message_result_callback(0, String::from("Transport Error")),
                        &rt_,
                    );

                    let content_type = b"message/cpim".to_vec();
                    let content_type = Some(content_type);

                    let size = message_body.estimated_size();

                    let mut data = Vec::with_capacity(size);
                    match message_body.reader() {
                        Ok(mut reader) => {
                            match reader.read_to_end(&mut data) {
                                Ok(_) => {}
                                Err(_) => {} // to-do: early failure
                            }
                        }
                        Err(_) => {} // to-do: early failure
                    }

                    let from_path = lpath.clone();
                    let to_path = rpath.clone();

                    let (chunk_tx, mut chunk_rx) = mpsc::channel(8);

                    demuxer.start(
                        from_path,
                        to_path,
                        content_type,
                        size,
                        VectorReader::new(data),
                        move |status_code, reason_phrase| {
                            message_result_callback_(status_code, reason_phrase)
                        },
                        chunk_tx,
                        &rt,
                    );

                    rt.spawn(async move {
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

                            match data_tx.send(Some(data)).await {
                                Ok(()) => {}
                                Err(e) => {}
                            }
                        }
                    });
                }
            });

            guard.10.replace(data_tx_);
        }
    }

    pub fn stop(&self, rt: &Arc<Runtime>) {
        let mut guard = self.state.lock().unwrap();
        if let Some(tx) = guard.10.take() {
            rt.spawn(async move {
                match tx.send(None).await {
                    Ok(()) => {}
                    Err(e) => {}
                }
            });
        }
        guard.2.take();
    }
}

struct StandaloneLargeModeNilReceiver {}

impl MsrpMessageReceiver for StandaloneLargeModeNilReceiver {
    fn on_message(
        &mut self,
        message_id: &[u8],
        content_type: &[u8],
    ) -> Result<Box<dyn MsrpDataWriter + Send + Sync>, (u16, &'static str)> {
        Err((415, "Unsupported Media Type"))
    }
}

struct StandaloneLargeModeReceiveSession {
    state: Arc<
        Mutex<(
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
            Option<mpsc::Sender<Option<Vec<u8>>>>,
        )>,
    >,

    contact_uri: Arc<Vec<u8>>,

    sip_cpim_body: Option<Arc<Body>>,
}

impl StandaloneLargeModeReceiveSession {
    pub fn new(
        l_sdp: Arc<Body>,
        r_sdp: Arc<Body>,
        cs: ClientSocket,
        tls: bool,
        laddr: String,
        lport: u16,
        lpath: Vec<u8>,
        raddr: String,
        rport: u16,
        rpath: Vec<u8>,
        contact_uri: Arc<Vec<u8>>,
        sip_cpim_body: Option<Arc<Body>>,
    ) -> StandaloneLargeModeReceiveSession {
        StandaloneLargeModeReceiveSession {
            state: Arc::new(Mutex::new((
                l_sdp,
                r_sdp,
                Some(cs),
                tls,
                laddr,
                lport,
                lpath,
                raddr,
                rport,
                rpath,
                None,
            ))),
            contact_uri,
            sip_cpim_body,
        }
    }

    pub fn start(
        &self,
        message_receive_listener: &Arc<
            dyn Fn(&[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync + 'static,
        >,
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
        rt: &Arc<Runtime>,
    ) {
        let mut guard = self.state.lock().unwrap();
        if let Some(sock) = guard.2.take() {
            let addr = String::from(&guard.7);
            let port = guard.8;
            let tls = guard.3;

            let lpath = guard.6.clone();
            let rpath = guard.9.clone();

            let sip_cpim_body = if let Some(sip_cpim_body) = &self.sip_cpim_body {
                Some(Arc::clone(sip_cpim_body))
            } else {
                None
            };

            let (data_tx, data_rx) = mpsc::channel(8);
            let data_tx_ = data_tx.clone();

            let message_receive_listener = Arc::clone(message_receive_listener);

            let connect_function = Arc::clone(&connect_function);

            let contact_uri = Arc::clone(&self.contact_uri);

            let rt_ = Arc::clone(rt);

            rt.spawn(async move {
                if let Ok(cs) = connect_function(sock, &addr, port, tls).await {
                    let t_id = format!("");

                    let from_path = lpath.clone();
                    let to_path = rpath.clone();

                    let demuxer = MsrpDemuxer::new(from_path, to_path);
                    let demuxer = Arc::new(demuxer);

                    let muxer = MsrpMuxer::new(StandaloneLargeModeReceiver {
                        message_receive_listener: Some(Box::new(
                            move |contact_uri, cpim_info, content_type, content_body| {
                                message_receive_listener(
                                    contact_uri,
                                    cpim_info,
                                    content_type,
                                    content_body,
                                )
                            },
                        )),
                        contact_uri,
                        sip_cpim_body,
                    });

                    let mut session_channel = MsrpChannel::new(lpath, rpath, demuxer, muxer);

                    msrp_transport_start(
                        cs,
                        t_id,
                        data_tx_,
                        data_rx,
                        move |message_chunk| session_channel.on_message(message_chunk),
                        || {},
                        &rt_,
                    );
                }
            });

            guard.10.replace(data_tx);
        }
    }

    pub fn stop(&self, rt: &Arc<Runtime>) {
        let mut guard = self.state.lock().unwrap();
        if let Some(tx) = guard.10.take() {
            rt.spawn(async move {
                match tx.send(None).await {
                    Ok(()) => {}
                    Err(e) => {}
                }
            });
        }
        guard.2.take();
    }
}

struct StandaloneLargeModeMessageDataWriter {
    data: Vec<u8>,
    contact_uri: Arc<Vec<u8>>,
    sip_cpim_body: Option<Arc<Body>>,
    message_receive_listener: Option<Box<dyn FnOnce(&[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync>>,
}

impl MsrpDataWriter for StandaloneLargeModeMessageDataWriter {
    fn write(&mut self, data: &[u8]) -> Result<usize, (u16, &'static str)> {
        self.data.extend_from_slice(data);
        Ok(data.len())
    }

    fn complete(&mut self) -> Result<(), (u16, &'static str)> {
        if let Some(message_receive_listener) = self.message_receive_listener.take() {
            match Body::construct_message(&self.data) {
                Ok(body) => match CPIMMessage::try_from(&body) {
                    Ok(cpim_message) => {
                        if let Some(sip_cpim_body) = &self.sip_cpim_body {
                            match CPIMMessage::try_from(sip_cpim_body) {
                                Ok(sip_cpim_message) => match sip_cpim_message.get_info() {
                                    Ok(sip_cpim_info) => {
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
                                                    message_receive_listener(
                                                        &self.contact_uri,
                                                        &sip_cpim_info,
                                                        content_type,
                                                        &decoded_content_body,
                                                    );
                                                    return Ok(());
                                                }
                                            } else {
                                                message_receive_listener(
                                                    &self.contact_uri,
                                                    &sip_cpim_info,
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
                            }
                        } else {
                            match cpim_message.get_info() {
                                Ok(cpim_info) => {
                                    if let Some((content_type, b, e)) =
                                        cpim_message.get_message_body()
                                    {
                                        let content_type: &[u8] = match content_type {
                                            Some(content_type) => content_type,
                                            None => b"text/plain",
                                        };

                                        if e {
                                            if let Ok(d) = general_purpose::STANDARD.decode(b) {
                                                message_receive_listener(
                                                    &self.contact_uri,
                                                    &cpim_info,
                                                    content_type,
                                                    &d,
                                                );
                                                return Ok(());
                                            }
                                        } else {
                                            message_receive_listener(
                                                &self.contact_uri,
                                                &cpim_info,
                                                content_type,
                                                b,
                                            );
                                            return Ok(());
                                        }
                                    }

                                    Err((400, "failed to get content body"))
                                }

                                Err(e) => Err((400, e)),
                            }
                        }
                    }
                    Err(e) => Err((400, e)),
                },
                Err(e) => Err((400, e)),
            }
        } else {
            Err((500, "message already received"))
        }
    }
}

struct StandaloneLargeModeReceiver {
    contact_uri: Arc<Vec<u8>>,
    sip_cpim_body: Option<Arc<Body>>,
    message_receive_listener: Option<Box<dyn FnOnce(&[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync>>,
}

impl MsrpMessageReceiver for StandaloneLargeModeReceiver {
    fn on_message(
        &mut self,
        message_id: &[u8],
        content_type: &[u8],
    ) -> Result<Box<dyn MsrpDataWriter + Send + Sync>, (u16, &'static str)> {
        if let Some(message_receive_listener) = self.message_receive_listener.take() {
            return Ok(Box::new(StandaloneLargeModeMessageDataWriter {
                data: Vec::with_capacity(1024),
                contact_uri: Arc::clone(&self.contact_uri),
                sip_cpim_body: match &self.sip_cpim_body {
                    Some(body) => Some(Arc::clone(body)),
                    None => None,
                },
                message_receive_listener: Some(message_receive_listener),
            }));
        }

        Err((500, ""))
    }
}

pub struct StandaloneMessagingService {
    registered_public_identity: Arc<Mutex<Option<(Arc<SipTransport>, String, String)>>>,
    message_receive_listener: Arc<dyn Fn(&[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync>,
    msrp_socket_allocator_function: Arc<
        dyn Fn(
                Option<&MsrpInfo>,
            )
                -> Result<(ClientSocket, String, u16, bool, bool, bool), (u16, &'static str)>
            + Send
            + Sync,
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
            + Sync,
    >,
}

impl StandaloneMessagingService {
    pub fn new<MRL, MAF, MCF>(
        message_receive_listener: MRL,
        msrp_socket_allocator_function: MAF,
        msrp_socket_connect_function: MCF,
    ) -> StandaloneMessagingService
    where
        MRL: Fn(&[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync + 'static,
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
    {
        StandaloneMessagingService {
            registered_public_identity: Arc::new(Mutex::new(None)),
            message_receive_listener: Arc::new(message_receive_listener),
            msrp_socket_allocator_function: Arc::new(msrp_socket_allocator_function),
            msrp_socket_connect_function: Arc::new(msrp_socket_connect_function),
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

    pub fn send_large_mode_message<MRF>(
        &self,
        message_type: &str,
        message_content: &str,
        recipient: &str,
        recipient_type: &RecipientType,
        recipient_uri: &str,
        message_result_callback: MRF,
        core: &Arc<SipCore>,
        rt: &Arc<Runtime>,
    ) where
        MRF: FnOnce(u16, String) + Send + Sync + 'static,
    {
        match (self.msrp_socket_allocator_function)(None) {
            Ok((sock, host, port, tls, active_setup, ipv6)) => {
                let path_random = create_raw_alpha_numeric_string(16);
                let path_random = std::str::from_utf8(&path_random).unwrap();
                let path = if tls {
                    format!("msrps://{}:{}/{};tcp", &host, port, path_random)
                } else {
                    format!("msrp://{}:{}/{};tcp", &host, port, path_random)
                };

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
                    direction: MsrpDirection::SendOnly,
                    accept_types: b"message/cpim",
                    setup_method: if active_setup {
                        MsrpSetupMethod::Active
                    } else {
                        MsrpSetupMethod::Passive
                    },
                    accept_wrapped_types: Some(message_type.as_bytes()),
                    file_info: None,
                };

                let sdp = String::from(msrp_info);
                let sdp_bytes = sdp.into_bytes();
                let sdp_body = Body::Raw(sdp_bytes);

                if let Some((transport, public_user_identity, instance_id)) =
                    core.get_default_public_identity()
                {
                    let mut invite_message =
                        SipMessage::new_request(INVITE, recipient_uri.as_bytes());

                    invite_message.add_header(Header::new(
                        b"Call-ID",
                        String::from(
                            Uuid::new_v4()
                                .as_hyphenated()
                                .encode_lower(&mut Uuid::encode_buffer()),
                        ),
                    ));

                    invite_message.add_header(Header::new(b"CSeq", b"1 INVITE"));

                    let tag = rand::create_raw_alpha_numeric_string(8);
                    let tag = String::from_utf8_lossy(&tag);

                    invite_message.add_header(Header::new(
                        b"From",
                        format!("<{}>;tag={}", public_user_identity, tag),
                    ));

                    invite_message.add_header(Header::new(b"To", format!("<{}>", recipient_uri)));

                    invite_message.add_header(Header::new(b"Contact", if let RecipientType::Chatbot = recipient_type {
                        format!("<{}>;+sip.instance=\"{}\";+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.largemsg\";+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot\";+g.gsma.rcs.botversion=\"#=1,#=2\"", &public_user_identity, &instance_id)
                    } else {
                        format!("<{}>;+sip.instance=\"{}\";+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.largemsg\"", &public_user_identity, &instance_id)
                    }));

                    invite_message.add_header(Header::new(b"Accept-Contact", format!("*;+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.largemsg\"")));

                    if message_type.eq_ignore_ascii_case("application/vnd.gsma.rcs-ft-http+xml") {
                        invite_message.add_header(Header::new(b"Accept-Contact", b"*;+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.fthttp\";require;explicit"));
                    }

                    // to-do: build up for other recipient types

                    if let RecipientType::Chatbot = recipient_type {
                        invite_message.add_header(Header::new(b"Accept-Contact", b"*;+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot.sa\";+g.gsma.rcs.botversion=\"#=1,#=2\";require;explicit"));
                    }

                    invite_message.add_header(Header::new(
                        b"Allow",
                        b"NOTIFY, OPTIONS, INVITE, UPDATE, CANCEL, BYE, ACK, MESSAGE",
                    ));

                    invite_message.add_header(Header::new(b"Supported", b"timer"));

                    invite_message.add_header(Header::new(b"Session-Expires", b"1800"));

                    invite_message.add_header(Header::new(
                        b"P-Preferred-Service",
                        b"urn:urn-7:3gpp-service.ims.icsi.oma.cpm.largemsg",
                    ));

                    invite_message.add_header(Header::new(
                        b"P-Preferred-Identity",
                        String::from(&public_user_identity),
                    ));

                    let message_imdn_id = Uuid::new_v4();

                    let cpim_body = make_cpim_message_content_body(
                        message_type,
                        "",
                        &recipient_type,
                        recipient_uri,
                        message_imdn_id,
                        &public_user_identity,
                    );

                    let boundary = create_raw_alpha_numeric_string(16);
                    let boundary_ = String::from_utf8_lossy(&boundary);
                    let mut parts =
                        Vec::with_capacity(if let RecipientType::ResourceList = recipient_type {
                            3
                        } else {
                            2
                        });

                    if let RecipientType::ResourceList = recipient_type {
                        let resource_list_body: Vec<u8> = recipient.as_bytes().to_vec();

                        let mut resource_list_headers = Vec::new();

                        let resource_list_length = resource_list_body.len();

                        resource_list_headers.push(Header::new(
                            b"Content-Type",
                            b"application/resource-lists+xml",
                        ));
                        resource_list_headers
                            .push(Header::new(b"Content-Disposition", b"recipient-list"));
                        resource_list_headers.push(Header::new(
                            b"Content-Length",
                            format!("{}", resource_list_length),
                        ));

                        let resource_list_body = Body::Message(MessageBody {
                            headers: resource_list_headers,
                            body: Arc::new(Body::Raw(resource_list_body)),
                        });

                        parts.push(Arc::new(resource_list_body));
                    }

                    let mut cpim_part_headers = Vec::new();
                    cpim_part_headers.push(Header::new(b"Content-Type", b"message/cpim"));
                    let cpim_content_length = cpim_body.estimated_size();
                    cpim_part_headers.push(Header::new(
                        b"Content-Length",
                        format!("{}", cpim_content_length),
                    ));

                    parts.push(Arc::new(Body::Message(MessageBody {
                        headers: cpim_part_headers,
                        body: Arc::new(cpim_body),
                    })));

                    let mut sdp_part_headers = Vec::new();
                    sdp_part_headers.push(Header::new(b"Content-Type", b"application/sdp"));
                    let sdp_content_length = sdp_body.estimated_size();
                    sdp_part_headers.push(Header::new(
                        b"Content-Length",
                        format!("{}", sdp_content_length),
                    ));

                    let sdp_body = Arc::new(sdp_body);
                    parts.push(Arc::new(Body::Message(MessageBody {
                        headers: sdp_part_headers,
                        body: Arc::clone(&sdp_body),
                    })));

                    invite_message.add_header(Header::new(
                        b"Content-Type",
                        format!("multipart/mixed; boundary={}", boundary_),
                    ));

                    let multipart = Body::Multipart(MultipartBody { boundary, parts });

                    let content_length = multipart.estimated_size();
                    invite_message.add_header(Header::new(
                        b"Content-Length",
                        format!("{}", content_length),
                    ));

                    let multipart = Arc::new(multipart);
                    invite_message.set_body(multipart);

                    let req_headers = invite_message.copy_headers();

                    let message_result_callback =
                        Arc::new(Mutex::new(Some(message_result_callback)));
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

                    let (client_transaction_tx, mut client_transaction_rx) =
                        mpsc::channel::<
                            Result<(SipMessage, CPMSessionInfo, Arc<Body>), (u16, String)>,
                        >(1);

                    let connect_function = Arc::clone(&self.msrp_socket_connect_function);

                    let ongoing_dialogs = core.get_ongoing_dialogs();

                    let core_ = Arc::clone(core);
                    let rt_ = Arc::clone(rt);

                    rt.spawn(async move {
                        if let Some(res) = client_transaction_rx.recv().await {
                            match res {
                                Ok((resp_message, session_info, r_sdp_body)) => {
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

                                                    let message_body = make_cpim_message_content_body(
                                                        &message.message_type,
                                                        &message.message_content,
                                                        &message.recipient_type,
                                                        &message.recipient_uri,
                                                        message_imdn_id,
                                                        &public_user_identity,
                                                    );
                                                    let message_body = Arc::new(message_body);

                                                    let session = StandaloneLargeModeSendSession::new(sdp_body, r_sdp_body, sock, tls, host, port, path, raddr, rport, rpath, message_body);

                                                    let session = Arc::new(session);

                                                    let (sess_tx, mut sess_rx) = mpsc::channel(8);

                                                    let sip_session = SipSession::new(&session, SipSessionEventReceiver {
                                                        tx: sess_tx,
                                                        rt: Arc::clone(&rt_),
                                                    });

                                                    let sip_session = Arc::new(sip_session);
                                                    let sip_session_ = Arc::clone(&sip_session);

                                                    let core = Arc::clone(&core_);
                                                    let rt = Arc::clone(&rt_);

                                                    rt_.spawn(async move {
                                                        let rt_ = Arc::clone(&rt);
                                                        let core_ = Arc::clone(&core);
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

                                                                        if let Some((transport, _, _)) = core_.get_default_public_identity() {
                                                                            core_.get_transaction_manager().send_request(
                                                                                message,
                                                                                &transport,
                                                                                UpdateMessageCallbacks {
                                                                                    // session_expires: None,
                                                                                    dialog,
                                                                                    sip_session: Arc::clone(&sip_session_),
                                                                                    rt: Arc::clone(&rt_),
                                                                                },
                                                                                &rt_,
                                                                            );
                                                                        }
                                                                    }
                                                                }

                                                                SipSessionEvent::Expired(dialog) => {
                                                                    if let Ok(message) = dialog.make_request(b"BYE", None) {

                                                                        if let Some((transport, _, _)) = core_.get_default_public_identity() {
                                                                            core_.get_transaction_manager().send_request(
                                                                                message,
                                                                                &transport,
                                                                                ClientTransactionNilCallbacks {},
                                                                                &rt_,
                                                                            );
                                                                        }
                                                                    }
                                                                }

                                                                _ => {}
                                                            }
                                                        }
                                                    });

                                                    sip_session.setup_confirmed_dialog(&dialog, StandaloneLargeModeSendSessionDialogEventReceiver {
                                                        sip_session: Arc::clone(&sip_session),
                                                        rt: Arc::clone(&rt_),
                                                    });

                                                    if let Ok(ack_message) = dialog.make_request(b"ACK", Some(1)) {

                                                        if let Some((transport, _, _)) = core_.get_default_public_identity(){

                                                            core_.get_transaction_manager().send_request(
                                                                ack_message,
                                                                &transport,
                                                                ClientTransactionNilCallbacks {},
                                                                &rt_,
                                                            );

                                                            session.start(&message.message_result_callback, &connect_function, &rt_);
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

                        (message.message_result_callback)(500, String::from("Internal Error"))
                    });

                    core.get_transaction_manager().send_request(
                        invite_message,
                        &transport,
                        StandaloneLargeModeInviteCallbacks {
                            recipient_type: RecipientType::from(recipient_type),
                            client_transaction_tx,
                            rt: Arc::clone(rt),
                        },
                        rt,
                    );

                    return;
                }

                message_result_callback(408, String::from("Timeout"));
            }

            Err((error_code, error_reason)) => {
                message_result_callback(error_code, String::from(error_reason))
            }
        }
    }
}

struct StandaloneLargeModeInviteCallbacks {
    recipient_type: RecipientType,
    client_transaction_tx:
        mpsc::Sender<Result<(SipMessage, CPMSessionInfo, Arc<Body>), (u16, String)>>,
    rt: Arc<Runtime>,
}

impl ClientTransactionCallbacks for StandaloneLargeModeInviteCallbacks {
    fn on_provisional_response(&self, _message: SipMessage) {}

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
        let client_transaction_tx = self.client_transaction_tx.clone();
        self.rt.spawn(async move {
            match client_transaction_tx
                .send(Err((0, String::from("Transport Error"))))
                .await
            {
                Ok(()) => {}
                Err(e) => {}
            }
        });
    }
}

pub struct StandaloneMessagingServiceWrapper {
    pub service: Arc<StandaloneMessagingService>,
    pub tm: Arc<SipTransactionManager>,
}

impl TransactionHandler for StandaloneMessagingServiceWrapper {
    fn handle_transaction(
        &self,
        transaction: &Arc<ServerTransaction>,
        ongoing_dialogs: &Arc<Mutex<Vec<Arc<SipDialog>>>>,
        channels: &mut Option<(
            mpsc::Sender<ServerTransactionEvent>,
            mpsc::Receiver<ServerTransactionEvent>,
        )>,
        // timer: &Timer,
        rt: &Arc<Runtime>,
    ) -> bool {
        platform_log(LOG_TAG, "handle_transaction");

        let message = transaction.message();
        if let SipMessage::Request(req_line, Some(req_headers), Some(req_body)) = message {
            if req_line.method == INVITE {
                platform_log(LOG_TAG, "is INVITE request");

                let mut is_large_mode_cpm_standalone_message = false;

                'check_large_mode: for header in
                    HeaderSearch::new(req_headers, b"Accept-Contact", true)
                {
                    let header_field = header.get_value().as_header_field();
                    if header_field
                        .contains_icsi_ref(b"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.largemsg")
                        || header_field.contains_icsi_ref(
                            b"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.deferred",
                        )
                    {
                        is_large_mode_cpm_standalone_message = true;
                        break 'check_large_mode;
                    }
                }

                if is_large_mode_cpm_standalone_message {
                    let mut conversation_id: &[u8] = &[];
                    let mut contribution_id: &[u8] = &[];

                    for header in req_headers {
                        if header.get_name().eq_ignore_ascii_case(b"Conversation-ID") {
                            conversation_id = header.get_value();
                        } else if header.get_name().eq_ignore_ascii_case(b"Contribution-ID") {
                            contribution_id = header.get_value();
                        }
                    }

                    if conversation_id.len() == 0 || contribution_id.len() == 0 {
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

                    let mut sdp: Option<Sdp> = None;
                    let mut sip_cpim_body: Option<Arc<Body>> = None;

                    if let Some(header) = search(req_headers, b"Content-Type", true) {
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
                            } else if content_type.major_type.eq_ignore_ascii_case(b"multipart")
                                && content_type.sub_type.eq_ignore_ascii_case(b"mixed")
                            {
                                match req_body.as_ref() {
                                    Body::Multipart(multipart) => {
                                        for part in &multipart.parts {
                                            if let Body::Message(body) = part.as_ref() {
                                                if let Some(header) =
                                                    search(&body.headers, b"Content-Type", true)
                                                {
                                                    if let Some(content_type) = header
                                                        .get_value()
                                                        .as_header_field()
                                                        .as_content_type()
                                                    {
                                                        if content_type
                                                            .major_type
                                                            .eq_ignore_ascii_case(b"application")
                                                            && content_type
                                                                .sub_type
                                                                .eq_ignore_ascii_case(b"sdp")
                                                        {
                                                            match body.body.as_ref() {
                                                                Body::Raw(raw) => {
                                                                    sdp = raw.as_sdp();
                                                                }
                                                                _ => {}
                                                            }
                                                        } else if content_type
                                                            .major_type
                                                            .eq_ignore_ascii_case(b"message")
                                                            && content_type
                                                                .sub_type
                                                                .eq_ignore_ascii_case(b"cpim")
                                                        {
                                                            sip_cpim_body =
                                                                Some(Arc::clone(&body.body));
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    _ => {}
                                }
                            }
                        }
                    }

                    if let Some(sdp) = sdp {
                        let mut contact_uri: Option<Vec<u8>> = None;

                        if let Some(header) = search(req_headers, b"P-Asserted-Identity", true) {
                            if let Some(asserted_identity) =
                                header.get_value().as_name_addresses().first()
                            {
                                if let Some(uri_part) = &asserted_identity.uri_part {
                                    let data_size = uri_part.estimated_size();
                                    let mut data = Vec::with_capacity(data_size);
                                    {
                                        let mut readers = Vec::new();
                                        uri_part.get_readers(&mut readers);
                                        match DynamicChain::new(readers).read_to_end(&mut data) {
                                            Ok(_) => {}
                                            Err(_) => {} // to-do: early failure
                                        }
                                    }

                                    contact_uri.replace(data);
                                }
                            }
                        }

                        if contact_uri.is_none() {
                            if let Some(sip_cpim_body) = &sip_cpim_body {
                                match CPIMMessage::try_from(sip_cpim_body) {
                                    Ok(sip_cpim_message) => {
                                        if let Ok(sip_cpim_info) = sip_cpim_message.get_info() {
                                            if let Some(from_uri) = sip_cpim_info.from_uri {
                                                contact_uri = Some(from_uri.string_representation_without_query_and_fragment());
                                            }
                                        }
                                    }
                                    Err(e) => {
                                        platform_log(LOG_TAG, format!("CPIM decode error: {}", e));
                                    }
                                }
                            }
                        }

                        if contact_uri.is_none() {
                            if let Some(header) = search(req_headers, b"Contact", true) {
                                if let Some(cpm_contact) =
                                    header.get_value().as_header_field().as_cpm_contact()
                                {
                                    contact_uri = Some(cpm_contact.service_uri.clone());
                                }
                            }
                        }

                        if let Some(contact_uri) = contact_uri {
                            let contact_uri = Arc::new(contact_uri);
                            if let Some(msrp_info) = sdp.as_msrp_info() {
                                match (self.service.msrp_socket_allocator_function)(Some(
                                    &msrp_info,
                                )) {
                                    Ok((sock, host, port, tls, active_setup, ipv6)) => {
                                        if let Ok(raddr) = std::str::from_utf8(msrp_info.address) {
                                            let raddr = String::from(raddr);
                                            let rport = msrp_info.port;
                                            let rpath = msrp_info.path.to_vec();

                                            let path_random = create_raw_alpha_numeric_string(16);
                                            let path_random =
                                                std::str::from_utf8(&path_random).unwrap();
                                            let path = if tls {
                                                format!(
                                                    "msrps://{}:{}/{};tcp",
                                                    &host, port, path_random
                                                )
                                            } else {
                                                format!(
                                                    "msrp://{}:{}/{};tcp",
                                                    &host, port, path_random
                                                )
                                            };

                                            let path = path.into_bytes();

                                            let l_msrp_info: MsrpInfo = MsrpInfo {
                                                protocol: if tls {
                                                    b"TCP/TLS/MSRP"
                                                } else {
                                                    b"TCP/MSRP"
                                                },
                                                address: host.as_bytes(),
                                                interface_type: if ipv6 {
                                                    MsrpInterfaceType::IPv6
                                                } else {
                                                    MsrpInterfaceType::IPv4
                                                },
                                                port,
                                                path: &path,
                                                inactive: false,
                                                direction: MsrpDirection::ReceiveOnly,
                                                accept_types: msrp_info.accept_types,
                                                setup_method: if active_setup {
                                                    MsrpSetupMethod::Active
                                                } else {
                                                    MsrpSetupMethod::Passive
                                                },
                                                accept_wrapped_types: msrp_info
                                                    .accept_wrapped_types,
                                                file_info: None,
                                            };

                                            let l_sdp = String::from(l_msrp_info);
                                            let l_sdp = l_sdp.into_bytes();
                                            let l_sdp = Body::Raw(l_sdp);
                                            let l_sdp = Arc::new(l_sdp);

                                            let r_sdp = Arc::clone(req_body);

                                            if let Some(mut resp_message) =
                                                server_transaction::make_response(
                                                    message,
                                                    transaction.to_tag(),
                                                    200,
                                                    b"Ok",
                                                )
                                            {
                                                let registered_public_identity = Arc::clone(
                                                    &self.service.registered_public_identity,
                                                );

                                                {
                                                    if let Some((
                                                        _,
                                                        contact_identity,
                                                        instance_id,
                                                    )) =
                                                        &*registered_public_identity.lock().unwrap()
                                                    {
                                                        resp_message.add_header(Header::new(
                                                            b"Contact",
                                                            format!("<{}>;+sip.instance=\"{}\";+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.largemsg\"", contact_identity, instance_id),
                                                        ));
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

                                                    let session =
                                                        StandaloneLargeModeReceiveSession::new(
                                                            l_sdp,
                                                            r_sdp,
                                                            sock,
                                                            tls,
                                                            host,
                                                            port,
                                                            path,
                                                            raddr,
                                                            rport,
                                                            rpath,
                                                            contact_uri,
                                                            sip_cpim_body,
                                                        );
                                                    let session = Arc::new(session);

                                                    let (sess_tx, mut sess_rx) = mpsc::channel(8);

                                                    let tm = Arc::clone(&self.tm);

                                                    let sip_session = SipSession::new(
                                                        &session,
                                                        SipSessionEventReceiver {
                                                            tx: sess_tx,
                                                            rt: Arc::clone(&rt),
                                                        },
                                                    );

                                                    let sip_session = Arc::new(sip_session);
                                                    let sip_session_ = Arc::clone(&sip_session);

                                                    let rt_ = Arc::clone(&rt);

                                                    rt.spawn(async move {
                                                        let rt = Arc::clone(&rt_);
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
                                                                        registered_public_identity.lock().unwrap();

                                                                        if let Some((transport, _, _)) = &*guard {
                                                                            tm.send_request(
                                                                                message,
                                                                                transport,
                                                                                UpdateMessageCallbacks {
                                                                                    // session_expires: None,
                                                                                    dialog,
                                                                                    sip_session: Arc::clone(&sip_session_),
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
                                                                        registered_public_identity.lock().unwrap();

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

                                                    sip_session.setup_confirmed_dialog(&dialog, StandaloneLargeModeReceiveSessionDialogEventReceiver {
                                                        sip_session: Arc::clone(&sip_session),
                                                        message_receive_listener: Arc::clone(&self.service.message_receive_listener),
                                                        connect_function: Arc::clone(&self.service.msrp_socket_connect_function),
                                                        rt: Arc::clone(&rt),
                                                    })
                                                }

                                                if let Some((tx, mut rx)) = channels.take() {
                                                    rt.spawn(async move{
                                                        while let Some(ev) = rx.recv().await {
                                                            match ev {
                                                                ServerTransactionEvent::Cancelled => {
                                                                    todo!()
                                                                },
                                                                _ => {},
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
                                                }
                                            }

                                            return true;
                                        }

                                        if let Some(resp_message) =
                                            server_transaction::make_response(
                                                message,
                                                transaction.to_tag(),
                                                400,
                                                b"Bad Request",
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
                                    }

                                    Err((error_code, error_phrase)) => {
                                        if let Some(resp_message) =
                                            server_transaction::make_response(
                                                message,
                                                transaction.to_tag(),
                                                error_code,
                                                error_phrase.as_bytes(),
                                            )
                                        {
                                            if let Some((tx, _rx)) = channels.take() {
                                                server_transaction::send_response(
                                                    Arc::clone(transaction),
                                                    resp_message,
                                                    tx,
                                                    // &timer,
                                                    rt,
                                                );
                                            }
                                        }
                                    }
                                }

                                return true;
                            }
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
            } else if req_line.method == MESSAGE {
                platform_log(LOG_TAG, "is MESSAGE request");

                if let Some(content_type_header) =
                    header::search(req_headers, b"Content-Type", true)
                {
                    let content_type_header_field =
                        content_type_header.get_value().as_header_field();
                    if let Some(content_type) = content_type_header_field.as_content_type() {
                        let mut is_imdn = false;
                        let mut cpim_message: Option<CPIMMessage> = None;

                        if content_type.major_type.equals_bytes(b"message", true)
                            && content_type.sub_type.equals_bytes(b"cpim", true)
                        {
                            match CPIMMessage::try_from(req_body) {
                                Ok(message) => {
                                    cpim_message = Some(message);
                                }
                                Err(e) => {
                                    platform_log(LOG_TAG, format!("CPIM decode error: {}", e));
                                }
                            }
                        }

                        if let Some(cpim_message) = &cpim_message {
                            is_imdn = cpim_message.contains_imdn();
                        }

                        let mut is_pager_mode_cpm_standalone_message = false;

                        'check_pager_mode: for header in
                            HeaderSearch::new(req_headers, b"Accept-Contact", true)
                        {
                            let header_field = header.get_value().as_header_field();
                            for parameter in header_field.get_parameter_iterator() {
                                if parameter.name == b"+g.3gpp.icsi-ref" {
                                    if let Some(value) = parameter.value {
                                        if let Some(_) = value.index_of(
                                            b"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.msg",
                                        ) {
                                            is_pager_mode_cpm_standalone_message = true;
                                            break 'check_pager_mode;
                                        } else if let Some(_) = value.index_of(
                                            b"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.deferred",
                                        ) {
                                            is_pager_mode_cpm_standalone_message = true;
                                            break 'check_pager_mode;
                                        } else if is_imdn {
                                            if let Some(_) = value.index_of(b"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.largemsg") {
                                                is_pager_mode_cpm_standalone_message = true;
                                                break 'check_pager_mode;
                                            } else if let Some(_) = value.index_of(b"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.session") {
                                                is_pager_mode_cpm_standalone_message = true;
                                                break 'check_pager_mode;
                                            } else if let Some(_) = value.index_of(b"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.filetransfer") {
                                                is_pager_mode_cpm_standalone_message = true;
                                                break 'check_pager_mode;
                                            }
                                        }
                                    }
                                } else if parameter.name == b"+g.3gpp.iari-ref" {
                                    if let Some(value) = parameter.value {
                                        if let Some(_) = value.index_of(
                                            b"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.fthttp",
                                        ) {
                                            is_pager_mode_cpm_standalone_message = true;
                                            break 'check_pager_mode;
                                        }
                                    }
                                }
                            }
                        }

                        if is_pager_mode_cpm_standalone_message {
                            platform_log(LOG_TAG, "is PAGER MODE");

                            let mut is_chatbot_message = false;

                            'check_chatbot: for header in
                                HeaderSearch::new(req_headers, b"Accept-Contact", true)
                            {
                                let header_field = header.get_value().as_header_field();
                                for parameter in header_field.get_parameter_iterator() {
                                    if parameter.name == b"+g.3gpp.iari-ref" {
                                        if let Some(value) = parameter.value {
                                            if let Some(_) = value.index_of(b"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot.sa") {
                                                is_chatbot_message = true;
                                                break 'check_chatbot;
                                            }
                                        }
                                    }
                                }
                            }

                            if is_chatbot_message {
                                // to-do: check chatbot parameters
                            }

                            if cpim_message.is_none() {
                                if let Body::Multipart(multipart) = req_body.as_ref() {
                                    platform_log(
                                        LOG_TAG,
                                        "trying to find CPIM in multipart messages",
                                    );
                                    for part in &multipart.parts {
                                        if let Body::Message(message) = part.as_ref() {
                                            if let Some(content_type_header) = header::search(
                                                &message.headers,
                                                b"Content-Type",
                                                false,
                                            ) {
                                                let content_type_header_field = content_type_header
                                                    .get_value()
                                                    .as_header_field();
                                                if let Some(content_type) =
                                                    content_type_header_field.as_content_type()
                                                {
                                                    if content_type
                                                        .major_type
                                                        .equals_bytes(b"message", true)
                                                        && content_type
                                                            .sub_type
                                                            .equals_bytes(b"cpim", true)
                                                    {
                                                        match CPIMMessage::try_from(req_body) {
                                                            Ok(cpim) => {
                                                                cpim_message = Some(cpim);
                                                                break;
                                                            }
                                                            Err(e) => {
                                                                platform_log(
                                                                    LOG_TAG,
                                                                    format!(
                                                                        "CPIM decode error: {}",
                                                                        e
                                                                    ),
                                                                );
                                                            }
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            if let Some(cpim_message) = cpim_message {
                                if let Ok(cpim_info) = cpim_message.get_info() {
                                    if !is_imdn && is_chatbot_message {
                                        if let None = cpim_info.get_bot_related_info() {
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
                                }

                                let mut contact_uri: Option<Vec<u8>> = None;

                                if let Some(p_asserted_identity_header) =
                                    header::search(req_headers, b"P-Asserted-Identity", true)
                                {
                                    if let Some(asserted_identity) = p_asserted_identity_header
                                        .get_value()
                                        .as_name_addresses()
                                        .first()
                                    {
                                        if let Some(uri_part) = &asserted_identity.uri_part {
                                            let data_size = uri_part.estimated_size();
                                            let mut data = Vec::with_capacity(data_size);
                                            {
                                                let mut readers = Vec::new();
                                                uri_part.get_readers(&mut readers);
                                                match DynamicChain::new(readers)
                                                    .read_to_end(&mut data)
                                                {
                                                    Ok(_) => {}
                                                    Err(_) => {} // to-do: early failure
                                                }
                                            }

                                            contact_uri.replace(data);
                                        }
                                    }
                                }

                                if contact_uri.is_none() {
                                    if let Ok(message_info) = cpim_message.get_info() {
                                        if let Some(uri) = message_info.from_uri {
                                            let from_uri = uri
                                                .string_representation_without_query_and_fragment();

                                            if let Some(from_uri) =
                                                from_uri.as_name_addresses().first()
                                            {
                                                if let Some(uri_part) = &from_uri.uri_part {
                                                    let data_size = uri_part.estimated_size();
                                                    let mut data = Vec::with_capacity(data_size);
                                                    {
                                                        let mut readers = Vec::new();
                                                        uri_part.get_readers(&mut readers);
                                                        match DynamicChain::new(readers)
                                                            .read_to_end(&mut data)
                                                        {
                                                            Ok(_) => {}
                                                            Err(_) => {} // to-do: early failure
                                                        }
                                                    }

                                                    contact_uri.replace(data);
                                                }
                                            }
                                        }
                                    }
                                }

                                if let Some(contact_uri) = contact_uri {
                                    if let Ok(message_info) = cpim_message.get_info() {
                                        if let Some((content_type, body, encoded)) =
                                            cpim_message.get_message_body()
                                        {
                                            let content_type: &[u8] = match content_type {
                                                Some(content_type) => content_type,
                                                None => match message_info.payload_type {
                                                    Some(payload_type) => payload_type,
                                                    None => b"text/plain",
                                                },
                                            };

                                            if encoded {
                                                if let Ok(decoded_body) =
                                                    general_purpose::STANDARD.decode(body)
                                                {
                                                    (self.service.message_receive_listener)(
                                                        &contact_uri,
                                                        &message_info,
                                                        content_type,
                                                        &decoded_body,
                                                    )
                                                }
                                            } else {
                                                (self.service.message_receive_listener)(
                                                    &contact_uri,
                                                    &message_info,
                                                    content_type,
                                                    body,
                                                )
                                            }
                                        }

                                        if let Some(resp_message) =
                                            server_transaction::make_response(
                                                message,
                                                transaction.to_tag(),
                                                200,
                                                b"Ok",
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
                                    } else {
                                        platform_log(LOG_TAG, "incomplete CPIM information");
                                    }
                                } else {
                                    platform_log(LOG_TAG, "cannot retrieve contact information")
                                }
                            } else {
                                platform_log(LOG_TAG, "cannot decode CPIM message");
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

        false
    }
}

pub fn send_message<F>(
    message_type: &str,
    message_content: &str,
    recipient: &str,
    recipient_type: &RecipientType,
    recipient_uri: &str,
    message_result_callback: F,
    core: Arc<SipCore>,
    transport: &Arc<SipTransport>,
    public_user_identity: &str,
    rt: &Arc<Runtime>,
) where
    F: FnOnce(u16, String) + Send + Sync + 'static,
{
    let mut req_message = SipMessage::new_request(MESSAGE, recipient_uri.as_bytes());

    req_message.add_header(Header::new(
        b"Call-ID",
        String::from(
            Uuid::new_v4()
                .as_hyphenated()
                .encode_lower(&mut Uuid::encode_buffer()),
        ),
    ));

    req_message.add_header(Header::new(b"CSeq", b"1 MESSAGE"));

    let tag = rand::create_raw_alpha_numeric_string(8);
    let tag = String::from_utf8_lossy(&tag);

    req_message.add_header(Header::new(
        b"From",
        format!("<{}>;tag={}", public_user_identity, tag),
    ));

    req_message.add_header(Header::new(b"To", format!("<{}>", recipient_uri)));

    req_message.add_header(Header::new(
        b"Accept-Contact",
        b"*;+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.msg\"",
    ));

    if message_type.eq_ignore_ascii_case("application/vnd.gsma.rcs-ft-http+xml") {
        req_message.add_header(Header::new(b"Accept-Contact", b"*;+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.fthttp\";require;explicit"));
    }

    if let RecipientType::Chatbot = recipient_type {
        req_message.add_header(Header::new(b"Accept-Contact", b"*;+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot.sa\";+g.gsma.rcs.botversion=\"#=1,#=2\";require;explicit"));
    }

    req_message.add_header(Header::new(
        b"Contribution-ID",
        String::from(
            Uuid::new_v4()
                .as_hyphenated()
                .encode_lower(&mut Uuid::encode_buffer()),
        ),
    ));

    req_message.add_header(Header::new(
        b"Conversation-ID",
        String::from(
            Uuid::new_v4()
                .as_hyphenated()
                .encode_lower(&mut Uuid::encode_buffer()),
        ),
    ));

    if let RecipientType::ResourceList = recipient_type {
        req_message.add_header(Header::new(b"Require", b"recipient-list-message"));

        req_message.add_header(Header::new(
            b"P-Preferred-Service",
            b"urn:urn-7:3gpp-service.ims.icsi.oma.cpm.msg.group",
        ));
    } else {
        req_message.add_header(Header::new(
            b"P-Preferred-Service",
            b"urn:urn-7:3gpp-service.ims.icsi.oma.cpm.msg",
        ));
    }

    req_message.add_header(Header::new(
        b"P-Preferred-Identity",
        String::from(public_user_identity),
    ));

    let cpim_body = make_cpim_message_content_body(
        message_type,
        message_content,
        &recipient_type,
        recipient_uri,
        Uuid::new_v4(),
        public_user_identity,
    );

    if let RecipientType::ResourceList = recipient_type {
        let boundary = create_raw_alpha_numeric_string(16);
        let boundary_ = String::from_utf8_lossy(&boundary);
        let mut parts = Vec::with_capacity(2);

        let resource_list_body: Vec<u8> = recipient.as_bytes().to_vec();

        let mut resource_list_body_headers = Vec::with_capacity(2);

        let resource_list_length = resource_list_body.len();

        resource_list_body_headers.push(Header::new(
            b"Content-Type",
            b"application/resource-lists+xml",
        ));
        resource_list_body_headers.push(Header::new(b"Content-Disposition", b"recipient-list"));
        resource_list_body_headers.push(Header::new(
            b"Content-Length",
            format!("{}", resource_list_length),
        ));

        let resource_list_body_part = Body::Message(MessageBody {
            headers: resource_list_body_headers,
            body: Arc::new(Body::Raw(resource_list_body)),
        });

        let mut cpim_body_headers = Vec::with_capacity(2);

        let cpim_body_len = cpim_body.estimated_size();

        cpim_body_headers.push(Header::new(b"Content-Type", "message/cpim"));
        cpim_body_headers.push(Header::new(b"Content-Length", format!("{}", cpim_body_len)));

        let cpim_body_part = Body::Message(MessageBody {
            headers: cpim_body_headers,
            body: Arc::new(cpim_body),
        });

        parts.push(Arc::new(resource_list_body_part));
        parts.push(Arc::new(cpim_body_part));

        req_message.add_header(Header::new(
            b"Content-Type",
            format!("multipart/mixed; boundary={}", boundary_),
        ));

        let sip_body = Body::Multipart(MultipartBody { boundary, parts });

        let sip_body_len = sip_body.estimated_size();

        req_message.set_body(Arc::new(sip_body));

        req_message.add_header(Header::new(b"Content-Length", format!("{}", sip_body_len)));
    } else {
        let sip_body_len = cpim_body.estimated_size();

        req_message.set_body(Arc::new(cpim_body));

        req_message.add_header(Header::new(b"Content-Type", b"message/cpim"));

        req_message.add_header(Header::new(b"Content-Length", format!("{}", sip_body_len)));
    };

    let (client_transaction_tx, mut client_transaction_rx) = mpsc::channel(1);

    rt.spawn(async move {
        if let Some((status_code, reason_phrase)) = client_transaction_rx.recv().await {
            message_result_callback(status_code, reason_phrase);
        }
    });

    core.get_transaction_manager().send_request(
        // ctrl_itf,
        req_message,
        transport,
        OutgoingPagerModeContext {
            client_transaction_tx,
            rt: Arc::clone(&rt),
        },
        &rt,
    );
}

struct StandaloneLargeModeSendSessionDialogEventReceiver {
    sip_session: Arc<SipSession<StandaloneLargeModeSendSession>>,
    rt: Arc<Runtime>,
}

impl SipDialogEventCallbacks for StandaloneLargeModeSendSessionDialogEventReceiver {
    fn on_ack(&self, _transaction: &Arc<ServerTransaction>) {}

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
        self.sip_session.get_inner().stop(&self.rt);
    }

    fn on_terminating_response(&self, _message: &SipMessage) {
        self.sip_session.get_inner().stop(&self.rt);
    }
}

struct StandaloneLargeModeReceiveSessionDialogEventReceiver {
    sip_session: Arc<SipSession<StandaloneLargeModeReceiveSession>>,
    message_receive_listener: Arc<dyn Fn(&[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync>,
    connect_function: Arc<
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
    rt: Arc<Runtime>,
}

impl SipDialogEventCallbacks for StandaloneLargeModeReceiveSessionDialogEventReceiver {
    fn on_ack(&self, _transaction: &Arc<ServerTransaction>) {
        self.sip_session.get_inner().start(
            &self.message_receive_listener,
            &self.connect_function,
            &self.rt,
        );
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

    fn on_terminating_request(&self, message: &SipMessage) {
        self.sip_session.get_inner().stop(&self.rt);
    }

    fn on_terminating_response(&self, message: &SipMessage) {
        self.sip_session.get_inner().stop(&self.rt);
    }
}

struct OutgoingPagerModeContext {
    client_transaction_tx: mpsc::Sender<(u16, String)>,
    rt: Arc<Runtime>,
}

impl ClientTransactionCallbacks for OutgoingPagerModeContext {
    fn on_provisional_response(&self, _message: SipMessage) {}

    fn on_final_response(&self, message: SipMessage) {
        if let SipMessage::Response(l, _, _) = message {
            let status_code = l.status_code;
            let reason_phrase = String::from_utf8_lossy(&l.reason_phrase).to_string();
            let tx = self.client_transaction_tx.clone();
            self.rt.spawn(async move {
                match tx.send((status_code, reason_phrase)).await {
                    Ok(()) => {}
                    Err(e) => {}
                }
            });
        }
    }

    fn on_transport_error(&self) {
        let tx = self.client_transaction_tx.clone();
        self.rt.spawn(async move {
            match tx.send((0, String::from("Transport Error"))).await {
                Ok(()) => {}
                Err(e) => {}
            }
        });
    }
}
