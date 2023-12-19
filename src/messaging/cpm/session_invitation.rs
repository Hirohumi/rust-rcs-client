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

use core::panic;
use std::{
    pin::Pin,
    sync::{Arc, Mutex},
};

use futures::Future;
use tokio::{runtime::Runtime, sync::mpsc};

use rust_rcs_core::{
    cpim::CPIMInfo,
    internet::{Body, Header},
    io::network::stream::{ClientSocket, ClientStream},
    msrp::info::{
        msrp_info_reader::AsMsrpInfo, MsrpDirection, MsrpInfo, MsrpInterfaceType, MsrpSetupMethod,
    },
    sip::{
        sip_core::SipDialogCache,
        sip_session::{
            choose_timeout_for_server_transaction_response, Refresher, SipSession, SipSessionEvent,
            SipSessionEventReceiver,
        },
        sip_transaction::{client_transaction::ClientTransactionNilCallbacks, server_transaction},
        ServerTransaction, ServerTransactionEvent, SipDialog, SipMessage, SipTransactionManager,
        SipTransport,
    },
    util::rand::create_raw_alpha_numeric_string,
};

use rust_strict_sdp::AsSDP;

use crate::contact::ContactKnownIdentities;

use super::{
    session::{CPMSession, CPMSessionDialogEventReceiver, CPMSessionInfo, UpdateMessageCallbacks},
    sip::cpm_contact::CPMServiceType,
};

pub enum CPMSessionInvitationResponse {
    Accept,
    Dispose,
}

pub struct CPMSessionInvitation {
    session_info: CPMSessionInfo,
    session_sdp: Arc<Body>,

    server_transaction: Arc<ServerTransaction>,

    tx: mpsc::Sender<ServerTransactionEvent>,
    rx: mpsc::Receiver<ServerTransactionEvent>,

    dialog: Arc<SipDialog>,
}

impl CPMSessionInvitation {
    pub fn new(
        session_info: CPMSessionInfo,
        session_sdp: Arc<Body>,
        server_transaction: Arc<ServerTransaction>,
        tx: mpsc::Sender<ServerTransactionEvent>,
        rx: mpsc::Receiver<ServerTransactionEvent>,
        dialog: Arc<SipDialog>,
    ) -> CPMSessionInvitation {
        CPMSessionInvitation {
            session_info,
            session_sdp,

            server_transaction,
            tx,
            rx,

            dialog,
        }
    }
}

pub fn try_accept_hanging_invitation(
    invitation: CPMSessionInvitation,
    msrp_socket_allocator_function: &Arc<
        dyn Fn(
                Option<&MsrpInfo>,
            )
                -> Result<(ClientSocket, String, u16, bool, bool, bool), (u16, &'static str)>
            + Send
            + Sync,
    >,
    msrp_socket_connect_function: &Arc<
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
    message_receive_listener: &Arc<
        dyn Fn(&Arc<SipSession<CPMSession>>, &[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync,
    >,
    registered_public_identity: &Arc<Mutex<Option<(Arc<SipTransport>, String, String)>>>,
    ongoing_dialogs: &Arc<Mutex<Vec<Arc<SipDialog>>>>,
    ongoing_sessions: &Arc<Mutex<Vec<(ContactKnownIdentities, Arc<SipSession<CPMSession>>)>>>,
    tm: &Arc<SipTransactionManager>,
    rt: &Arc<Runtime>,
) {
    let session_info = invitation.session_info;
    let session_sdp = invitation.session_sdp;

    let transaction = invitation.server_transaction;

    let message = transaction.message();

    let tx = invitation.tx;
    let rx = invitation.rx;

    let dialog = Some(invitation.dialog);

    if let Body::Raw(body) = session_sdp.as_ref() {
        if let Some(sdp) = body.as_sdp() {
            if let Some(msrp_info) = sdp.as_msrp_info() {
                try_accept_invitation(
                    session_info,
                    tx,
                    rx,
                    dialog,
                    &session_sdp,
                    msrp_info,
                    msrp_socket_allocator_function,
                    msrp_socket_connect_function,
                    message_receive_listener,
                    &transaction,
                    message,
                    registered_public_identity,
                    ongoing_dialogs,
                    ongoing_sessions,
                    tm,
                    rt,
                );

                return;
            }
        }
    }

    if let Some(resp_message) = server_transaction::make_response(
        message,
        transaction.to_tag(),
        500,
        b"Server Internal Error",
    ) {
        server_transaction::send_response(
            transaction,
            resp_message,
            tx,
            // &timer,
            rt,
        );
    }
}

pub fn try_accept_invitation<'a, 'b>(
    session_info: CPMSessionInfo,
    tx: mpsc::Sender<ServerTransactionEvent>,
    mut rx: mpsc::Receiver<ServerTransactionEvent>,
    dialog: Option<Arc<SipDialog>>,
    r_sdp: &'a Arc<Body>,
    msrp_info: MsrpInfo<'a>,
    msrp_socket_allocator_function: &Arc<
        dyn Fn(
                Option<&MsrpInfo>,
            )
                -> Result<(ClientSocket, String, u16, bool, bool, bool), (u16, &'static str)>
            + Send
            + Sync,
    >,
    msrp_socket_connect_function: &Arc<
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
    message_receive_listener: &Arc<
        dyn Fn(&Arc<SipSession<CPMSession>>, &[u8], &CPIMInfo, &[u8], &[u8]) + Send + Sync,
    >,
    transaction: &'b Arc<ServerTransaction>,
    message: &'b SipMessage,
    registered_public_identity: &Arc<Mutex<Option<(Arc<SipTransport>, String, String)>>>,
    ongoing_dialogs: &Arc<Mutex<Vec<Arc<SipDialog>>>>,
    ongoing_sessions: &Arc<Mutex<Vec<(ContactKnownIdentities, Arc<SipSession<CPMSession>>)>>>,
    tm: &Arc<SipTransactionManager>,
    rt: &Arc<Runtime>,
) {
    match (msrp_socket_allocator_function)(Some(&msrp_info)) {
        Ok((cs, host, port, tls, active_setup, ipv6)) => {
            if let Ok(raddr) = std::str::from_utf8(msrp_info.address) {
                let raddr = String::from(raddr);
                let rport = msrp_info.port;
                let rpath = msrp_info.path.to_vec();

                let path_random = create_raw_alpha_numeric_string(16);
                let path_random = std::str::from_utf8(&path_random).unwrap();
                let path = if tls {
                    format!("msrps://{}:{}/{};tcp", &host, port, path_random)
                } else {
                    format!("msrp://{}:{}/{};tcp", &host, port, path_random)
                };

                let path = path.into_bytes();

                let l_msrp_info: MsrpInfo = MsrpInfo { protocol: if tls {
                    b"TCP/TLS/MSRP"
                } else {
                    b"TCP/MSRP"
                }, address: host.as_bytes(), interface_type: if ipv6 {
                    MsrpInterfaceType::IPv6
                } else {
                    MsrpInterfaceType::IPv4
                }, port, path: &path, inactive: false, direction: MsrpDirection::SendReceive, accept_types: match session_info.cpm_contact.service_type {
                    CPMServiceType::OneToOne => {
                        b"message/cpim application/im-iscomposing+xm"
                    },
                    CPMServiceType::Group => {
                        b"message/cpim application/conference-info+xml"
                    },
                    CPMServiceType::Chatbot => {
                        b"message/cpim"
                    },
                    CPMServiceType::System => {
                        b"message/cpim"
                    },
                }, setup_method: if active_setup {
                    MsrpSetupMethod::Active
                } else {
                    MsrpSetupMethod::Passive
                }, accept_wrapped_types: match session_info.cpm_contact.service_type {
                    CPMServiceType::OneToOne => {
                        Some(b"multipart/mixed text/plain application/vnd.gsma.rcs-ft-http+xml application/vnd.gsma.rcspushlocation+xml message/imdn+xml")
                    },
                    CPMServiceType::Group => {
                        Some(b"multipart/mixed text/plain application/vnd.gsma.rcs-ft-http+xml application/vnd.gsma.rcspushlocation+xml message/imdn+xml application/im-iscomposing+xml")
                    },
                    CPMServiceType::Chatbot => {
                        Some(b"multipart/mixed text/plain application/vnd.gsma.rcs-ft-http+xml application/vnd.gsma.rcspushlocation+xml message/imdn+xml application/vnd.gsma.botmessage.v1.0+json application/vnd.gsma.botsuggestion.v1.0+json application/commontemplate+xml")
                    },
                    CPMServiceType::System => {
                        Some(b"multipart/mixed text/plain application/vnd.gsma.rcs-ft-http+xml application/vnd.gsma.rcspushlocation+xml message/imdn+xml")
                    },
                }, file_info: None };

                let l_sdp = String::from(l_msrp_info);
                let l_sdp = l_sdp.into_bytes();
                let l_sdp = Body::Raw(l_sdp);
                let l_sdp = Arc::new(l_sdp);

                let known_identities = session_info.get_contact_known_identities();

                let contact_uri = session_info.asserted_contact_uri.as_bytes().to_vec();
                let session = CPMSession::new(session_info);
                let session = Arc::new(session);

                let r_sdp = Arc::clone(r_sdp);
                if let Ok(_) = session.set_remote_sdp(r_sdp, raddr, rport, rpath) {
                    if let Ok(_) = session.set_local_sdp(l_sdp, cs, tls, host, port, path) {
                        if let Some(mut resp_message) = server_transaction::make_response(
                            message,
                            transaction.to_tag(),
                            200,
                            b"OK",
                        ) {
                            let public_user_identity = {
                                let guard = registered_public_identity.lock().unwrap();

                                if let Some((transport, contact_identity, instance_id)) = &*guard {
                                    let transport_ = transaction.transport();
                                    if Arc::ptr_eq(transport, transport_) {
                                        resp_message.add_header(Header::new(
                                            b"Contact",
                                            match session.get_session_info().cpm_contact.service_type {
                                                CPMServiceType::Chatbot => {
                                                    format!("<{}>;+sip.instance=\"{}\";+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.session\";+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot\";+g.gsma.rcs.botversion=\"#=1,#=2\"", contact_identity, instance_id)
                                                },
                                                _ => {
                                                    format!("<{}>;+sip.instance=\"{}\";+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.session\"", contact_identity, instance_id)
                                                },
                                            },
                                        ));

                                        String::from(contact_identity)
                                    } else {
                                        panic!("")
                                    }
                                } else {
                                    panic!("")
                                }
                            };

                            let dialog = if let Some(dialog) = dialog {
                                dialog
                            } else {
                                let (d_tx, mut d_rx) = tokio::sync::mpsc::channel(1);

                                let ongoing_dialogs_ = Arc::clone(ongoing_dialogs);

                                rt.spawn(async move {
                                    if let Some(dialog) = d_rx.recv().await {
                                        ongoing_dialogs_.remove_dialog(&dialog);
                                    }
                                });

                                if let Ok(dialog) =
                                    SipDialog::try_new_as_uas(message, &resp_message, move |d| {
                                        match d_tx.blocking_send(d) {
                                            Ok(()) => {}
                                            Err(e) => {}
                                        }
                                    })
                                {
                                    let dialog = Arc::new(dialog);

                                    ongoing_dialogs.add_dialog(&dialog);

                                    dialog
                                } else {
                                    panic!("")
                                }
                            };

                            let (sess_tx, mut sess_rx) = mpsc::channel::<SipSessionEvent>(8);

                            let sip_session = SipSession::new(
                                &session,
                                SipSessionEventReceiver {
                                    tx: sess_tx,
                                    rt: Arc::clone(rt),
                                },
                            );

                            let sip_session = Arc::new(sip_session);
                            let sip_session_ = Arc::clone(&sip_session);

                            let message_receive_listener = Arc::clone(message_receive_listener);

                            sip_session.setup_confirmed_dialog(
                                &dialog,
                                CPMSessionDialogEventReceiver {
                                    public_user_identity,
                                    sip_session: Arc::clone(&sip_session),
                                    ongoing_sessions: Arc::clone(ongoing_sessions),
                                    message_receive_listener: Arc::new(
                                        move |cpim_info, content_type, content_body| {
                                            message_receive_listener(
                                                &sip_session_,
                                                &contact_uri,
                                                cpim_info,
                                                content_type,
                                                content_body,
                                            )
                                        },
                                    ),
                                    msrp_socket_connect_function: Arc::clone(
                                        msrp_socket_connect_function,
                                    ),
                                    rt: Arc::clone(rt),
                                },
                            );

                            match choose_timeout_for_server_transaction_response(
                                transaction,
                                true,
                                Refresher::UAS,
                            ) {
                                Ok(Some((timeout, refresher))) => match refresher {
                                    Refresher::UAC => {
                                        resp_message.add_header(Header::new(
                                            b"Session-Expires",
                                            format!("{};refresher=uac", timeout),
                                        ));

                                        sip_session.schedule_refresh(timeout, false, rt);
                                    }
                                    Refresher::UAS => {
                                        resp_message.add_header(Header::new(
                                            b"Session-Expires",
                                            format!("{};refresher=uas", timeout),
                                        ));

                                        sip_session.schedule_refresh(timeout, true, rt);
                                    }
                                },

                                Ok(None) => {}

                                Err((error_code, _error_phrase, min_se)) => {
                                    if error_code == 422 {
                                        // to-do: provide a larger expire value
                                    }
                                }
                            }

                            let sip_session_ = Arc::clone(&sip_session);

                            let registered_public_identity = Arc::clone(registered_public_identity);

                            let tm = Arc::clone(tm);
                            let rt_ = Arc::clone(rt);

                            rt.spawn(async move {
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
                                                            rt: Arc::clone(&rt_),
                                                        },
                                                        &rt_,
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
                                                        &rt_,
                                                    );
                                                }
                                            }
                                        }

                                        _ => {}
                                    }
                                }
                            });

                            ongoing_sessions
                                .lock()
                                .unwrap()
                                .push((known_identities, sip_session));

                            // let msrp_socket_connect_function =
                            //     Arc::clone(msrp_socket_connect_function);
                            // let rt_ = Arc::clone(rt);
                            rt.spawn(async move {
                                while let Some(ev) = rx.recv().await {
                                    match ev {
                                        ServerTransactionEvent::Acked => {
                                            // session.on_ack(&msrp_socket_connect_function, &rt_)
                                        }
                                        _ => {}
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

                        return;
                    }
                }
            }

            if let Some(resp_message) = server_transaction::make_response(
                message,
                transaction.to_tag(),
                400,
                b"Bad Request",
            ) {
                server_transaction::send_response(
                    Arc::clone(transaction),
                    resp_message,
                    tx,
                    // &timer,
                    rt,
                );
            }
        }

        Err((error_code, error_phrase)) => {
            if let Some(resp_message) = server_transaction::make_response(
                message,
                transaction.to_tag(),
                error_code,
                error_phrase.as_bytes(),
            ) {
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
