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
    ffi::CStr,
    ptr::NonNull,
    sync::{Arc, Mutex},
};

use futures::channel::oneshot;
use tokio::{runtime::Runtime, sync::mpsc};

use uuid::Uuid;

use rust_rcs_core::{
    internet::{
        body::{message_body::MessageBody, multipart_body::MultipartBody},
        header,
        headers::AsContentType,
        AsHeaderField, Body, Header,
    },
    io::Serializable,
    sip::{
        sip_core::SipDialogCache,
        sip_session::{SipSession, SipSessionEvent, SipSessionEventReceiver},
        sip_transaction::{client_transaction::ClientTransactionNilCallbacks, server_transaction},
        ClientTransactionCallbacks, ServerTransaction, ServerTransactionEvent, SipCore, SipDialog,
        SipDialogEventCallbacks, SipMessage, SipTransactionManager, SipTransport,
        TransactionHandler, INVITE,
    },
    util::rand::create_raw_alpha_numeric_string,
};

use crate::messaging::cpm::session::UpdateMessageCallbacks;

use self::{
    ffi::{
        MultiConferenceEventListener, MultiConferenceEventListenerContextWrapper,
        MultiConferenceV1InviteResponse,
    },
    sip::AsConferenceV1Contact,
    subscription::MultiConferenceEvent,
};

pub mod ffi;
pub mod sip;
pub mod subscription;

struct MultiConferenceV1Session {
    conference_id: String,
    // sdp: String,
    tx: mpsc::Sender<MultiConferenceEvent>,
    // event_listener: Arc<
    //     Mutex<
    //         Option<(
    //             MultiConferenceEventCallback,
    //             MultiConferenceEventCallbackContextWrapper,
    //         )>,
    //     >,
    // >,
}

impl MultiConferenceV1Session {
    pub fn get_conference_id(&self) -> String {
        self.conference_id.clone()
    }
    // pub fn get_sdp(&self) -> String {
    //     self.sdp.clone()
    // }

    pub fn start(
        &self,
        mut rx: mpsc::Receiver<MultiConferenceEvent>,
        event_cb: MultiConferenceEventListener,
        event_cb_context: Arc<Mutex<MultiConferenceEventListenerContextWrapper>>,
        rt: &Arc<Runtime>,
    ) {
        rt.spawn(async move {
            while let Some(ev) = rx.recv().await {
                let ev_type = ev.get_event_type();
                let ev = Some(Box::new(ev));
                let event_cb_context = event_cb_context.lock().unwrap();
                let cb_context = event_cb_context.0.as_ptr();
                event_cb(ev_type, ev, cb_context);
            }
        });
    }
}

struct MultiConferenceV1SessionDialogEventReceiver {
    sip_session: Arc<SipSession<MultiConferenceV1Session>>,
    event_listener: Arc<
        Mutex<
            Option<(
                mpsc::Receiver<MultiConferenceEvent>,
                MultiConferenceEventListener,
                MultiConferenceEventListenerContextWrapper,
            )>,
        >,
    >,
    rt: Arc<Runtime>,
}

// MultiConferenceV1 server does not actively initiate requests
impl SipDialogEventCallbacks for MultiConferenceV1SessionDialogEventReceiver {
    fn on_ack(&self, _transaction: &Arc<ServerTransaction>) {
        let mut guard = self.event_listener.lock().unwrap();
        if let Some((rx, event_cb, event_cb_context)) = guard.take() {
            let session = self.sip_session.get_inner();

            let event_cb_context = Arc::new(Mutex::new(event_cb_context));

            session.start(rx, event_cb, event_cb_context, &self.rt);
        }
    }

    fn on_new_request(
        &self,
        _transaction: Arc<ServerTransaction>,
        _tx: mpsc::Sender<ServerTransactionEvent>,
        _rt: &Arc<Runtime>,
    ) -> Option<(u16, bool)> {
        None
    }

    fn on_terminating_request(&self, _message: &SipMessage) {
        let inner = self.sip_session.get_inner();

        let tx = inner.tx.clone();

        self.rt.spawn(async move {
            match tx.send(MultiConferenceEvent::conference_end()).await {
                Ok(()) => {}
                Err(e) => {}
            }
        });
    }

    fn on_terminating_response(&self, _message: &SipMessage) {
        let inner = self.sip_session.get_inner();

        let tx = inner.tx.clone();

        self.rt.spawn(async move {
            match tx.send(MultiConferenceEvent::conference_end()).await {
                Ok(()) => {}
                Err(e) => {}
            }
        });
    }
}

struct MultiConferenceV1Internal {
    sip_session: Arc<SipSession<MultiConferenceV1Session>>,
    // cb: MultiConferenceEventCallback,
    // cb_context : MultiConferenceEventCallbackContextWrapper,
}

impl MultiConferenceV1Internal {
    // pub fn get_sdp(&self) -> String {
    //     let session = self.sip_session.get_inner();
    //     session.get_sdp()
    // }
    pub fn keep_alive(&self, _rt: &Arc<Runtime>) {
        self.sip_session.mark_session_active()
    }
}

pub struct MultiConferenceV1 {
    inner: Arc<MultiConferenceV1Internal>,
}

// impl MultiConferenceV1 {
//     pub fn register_event_listener(
//         &mut self,
//         cb: MultiConferenceEventCallback,
//         cb_context: MultiConferenceEventCallbackContextWrapper,
//     ) {
//         let inner = self.inner.sip_session.get_inner();

//         let mut guard = inner.event_listener.lock().unwrap();

//         *guard = Some((cb, cb_context));
//     }
// }

impl MultiConferenceV1 {
    // pub fn get_sdp(&self) -> String {
    //     self.inner.get_sdp()
    // }
    pub fn keep_alive(&self, rt: &Arc<Runtime>) {
        self.inner.keep_alive(rt)
    }
}

/**
 * conference v1 is set to use with go-rcs-server and rcs-mediasoup-server
 */
pub struct MultiConferenceServiceV1 {
    multi_conference_invite_handler: Arc<
        dyn Fn(MultiConferenceV1, String, MultiConferenceV1InviteResponseReceiver) + Send + Sync,
    >,

    registered_public_identity: Arc<Mutex<Option<(Arc<SipTransport>, String, String)>>>,
}

impl MultiConferenceServiceV1 {
    pub fn new<MCIHF>(multi_conference_invite_handler_function: MCIHF) -> MultiConferenceServiceV1
    where
        MCIHF: Fn(MultiConferenceV1, String, MultiConferenceV1InviteResponseReceiver)
            + Send
            + Sync
            + 'static,
    {
        MultiConferenceServiceV1 {
            multi_conference_invite_handler: Arc::new(multi_conference_invite_handler_function),

            registered_public_identity: Arc::new(Mutex::new(None)),
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

    pub fn create_conference<F>(
        &self,
        recipients: &str,
        offer_sdp: &str,
        event_cb: Option<MultiConferenceEventListener>,
        event_cb_context: MultiConferenceEventListenerContextWrapper,
        core: &Arc<SipCore>,
        rt: &Arc<Runtime>,
        callback: F,
    ) where
        F: FnOnce(Option<(MultiConferenceV1, String)>) + Send + Sync + 'static,
    {
        let event_cb_context = Arc::new(Mutex::new(event_cb_context));

        if let Some((transport, public_user_identity, instance_id)) =
            core.get_default_public_identity()
        {
            let mut invite_message =
                SipMessage::new_request(INVITE, b"sip:conference-focus@example.com");

            invite_message.add_header(Header::new(
                b"Call-ID",
                String::from(
                    Uuid::new_v4()
                        .as_hyphenated()
                        .encode_lower(&mut Uuid::encode_buffer()),
                ),
            ));

            invite_message.add_header(Header::new(b"CSeq", b"1 INVITE"));

            let tag = create_raw_alpha_numeric_string(8);
            let tag = String::from_utf8_lossy(&tag);

            invite_message.add_header(Header::new(b"From", format!("<{}>;", public_user_identity)));

            invite_message.add_header(Header::new(b"To", b"<sip:conference-focus@example.com>"));

            let boundary = create_raw_alpha_numeric_string(16);
            let boundary_ = String::from_utf8_lossy(&boundary);
            let mut parts = Vec::with_capacity(2);

            let mut recipients_part_headers = Vec::new();
            recipients_part_headers.push(Header::new(b"Content-Type", b"application/json"));
            let recipients_json_length = recipients.len();
            recipients_part_headers.push(Header::new(
                b"Content-Length",
                format!("{}", recipients_json_length),
            ));

            parts.push(Arc::new(Body::Message(MessageBody {
                headers: recipients_part_headers,
                body: Arc::new(Body::Raw(recipients.as_bytes().to_vec())),
            })));

            let mut sdp_part_headers = Vec::new();
            sdp_part_headers.push(Header::new(b"Content-Type", b"application/sdp"));
            let sdp_content_length = offer_sdp.len();
            sdp_part_headers.push(Header::new(
                b"Content-Length",
                format!("{}", sdp_content_length),
            ));

            parts.push(Arc::new(Body::Message(MessageBody {
                headers: sdp_part_headers,
                body: Arc::new(Body::Raw(offer_sdp.as_bytes().to_vec())),
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

            let (client_transaction_tx, mut client_transaction_rx) = mpsc::channel(1);

            let ongoing_dialogs = core.get_ongoing_dialogs();

            let core_ = Arc::clone(&core);
            let rt_ = Arc::clone(rt);

            rt.spawn(async move {
                if let Some(res) = client_transaction_rx.recv().await {
                    match res {
                        Ok((resp_message, conference_id, sdp)) => {
                            let (d_tx, mut d_rx) = mpsc::channel(1);

                            let ongoing_dialogs_ = Arc::clone(&ongoing_dialogs);

                            rt_.spawn(async move {
                                if let Some(dialog) = d_rx.recv().await {
                                    ongoing_dialogs_.remove_dialog(&dialog);
                                }
                            });

                            if let Ok(dialog) =
                                SipDialog::try_new_as_uac(&req_headers, &resp_message, move |d| {
                                    match d_tx.blocking_send(d) {
                                        Ok(()) => {}
                                        Err(e) => {}
                                    }
                                })
                            {
                                let dialog = Arc::new(dialog);

                                ongoing_dialogs.add_dialog(&dialog);

                                let (conf_tx, conf_rx) = mpsc::channel(8);

                                let session = MultiConferenceV1Session {
                                    conference_id,
                                    tx: conf_tx,
                                };
                                let session = Arc::new(session);

                                let (sess_tx, mut sess_rx) = mpsc::channel(8);

                                let sip_session = SipSession::new(
                                    &session,
                                    SipSessionEventReceiver {
                                        tx: sess_tx,
                                        rt: Arc::clone(&rt_),
                                    },
                                );

                                let sip_session = Arc::new(sip_session);
                                let sip_session_ = Arc::clone(&sip_session);

                                let core = Arc::clone(&core_);
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

                                                    if let Some((transport, _, _)) =
                                                        core.get_default_public_identity()
                                                    {
                                                        core.get_transaction_manager()
                                                            .send_request(
                                                                message,
                                                                &transport,
                                                                UpdateMessageCallbacks {
                                                                    // session_expires: None,
                                                                    dialog,
                                                                    sip_session: Arc::clone(
                                                                        &sip_session_,
                                                                    ),
                                                                    rt: Arc::clone(&rt),
                                                                },
                                                                &rt,
                                                            );
                                                    }
                                                }
                                            }

                                            SipSessionEvent::Expired(dialog) => {
                                                if let Ok(message) =
                                                    dialog.make_request(b"BYE", None)
                                                {
                                                    if let Some((transport, _, _)) =
                                                        core.get_default_public_identity()
                                                    {
                                                        core.get_transaction_manager()
                                                            .send_request(
                                                                message,
                                                                &transport,
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

                                sip_session.setup_confirmed_dialog(
                                    &dialog,
                                    MultiConferenceV1SessionDialogEventReceiver {
                                        sip_session: Arc::clone(&sip_session),
                                        event_listener: Arc::new(Mutex::new(None)),
                                        rt: Arc::clone(&rt_),
                                    },
                                );

                                if let Ok(ack_message) = dialog.make_request(b"ACK", Some(1)) {
                                    if let Some((transport, _, _)) =
                                        core_.get_default_public_identity()
                                    {
                                        core_.get_transaction_manager().send_request(
                                            ack_message,
                                            &transport,
                                            ClientTransactionNilCallbacks {},
                                            &rt_,
                                        );
                                    }
                                }

                                // to-do: start progressing events

                                if let Some(event_cb) = event_cb {
                                    session.start(conf_rx, event_cb, event_cb_context, &rt_);
                                }

                                // rt_.spawn(async move {
                                //     while let Some(ev) = conf_rx.recv().await {
                                //         if let Some(cb) = event_cb {
                                //             let ev = Some(Box::new(ev));
                                //             let event_cb_context = event_cb_context.lock().unwrap();
                                //             let cb_context = event_cb_context.0.as_ptr();
                                //             cb(ev, cb_context);
                                //         }
                                //     }
                                // });

                                // to-do: setup subscriber

                                let inner = MultiConferenceV1Internal { sip_session };
                                let inner = Arc::new(inner);

                                callback(Some((MultiConferenceV1 { inner }, sdp)));

                                return;
                            }

                            callback(None)
                        }

                        Err(e) => callback(None),
                    }
                }
            });

            core.get_transaction_manager().send_request(
                invite_message,
                &transport,
                ConferenceV1InviteCallbacks {
                    client_transaction_tx,
                    rt: Arc::clone(rt),
                },
                rt,
            );

            return;
        }

        callback(None)
    }
}

struct ConferenceV1InviteCallbacks {
    client_transaction_tx: mpsc::Sender<Result<(SipMessage, String, String), u16>>,
    rt: Arc<Runtime>,
}

impl ClientTransactionCallbacks for ConferenceV1InviteCallbacks {
    fn on_provisional_response(&self, _message: SipMessage) {}

    fn on_final_response(&self, message: SipMessage) {
        if let SipMessage::Response(l, headers, _) = &message {
            if l.status_code >= 200 && l.status_code < 300 {
                if let Some(headers) = headers {
                    if let Some(contact_header) = header::search(headers, b"Contact", true) {
                        if let Some(conference_v1_contact) = contact_header
                            .get_value()
                            .as_header_field()
                            .as_conference_v1_contact()
                        {
                            if let Some(content_type_header) =
                                header::search(headers, b"Content-Type", true)
                            {
                                if let Some(content_type) = content_type_header
                                    .get_value()
                                    .as_header_field()
                                    .as_content_type()
                                {
                                    if content_type.major_type.eq_ignore_ascii_case(b"application")
                                        && content_type.sub_type.eq_ignore_ascii_case(b"sdp")
                                    {
                                        if let Some(body) = message.get_body() {
                                            if let Body::Raw(bytes) = body.as_ref() {
                                                if let Ok(sdp) = std::str::from_utf8(bytes) {
                                                    let sdp = String::from(sdp);

                                                    let tx = self.client_transaction_tx.clone();

                                                    self.rt.spawn(async move {
                                                        match tx
                                                            .send(Ok((
                                                                message,
                                                                conference_v1_contact
                                                                    .conference_uri,
                                                                sdp,
                                                            )))
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
                            }

                            return;
                        }
                    }

                    let tx = self.client_transaction_tx.clone();

                    self.rt.spawn(async move {
                        match tx.send(Err(400)).await {
                            Ok(()) => {}
                            Err(e) => {}
                        }
                    });

                    return;
                }

                let code = l.status_code;

                let tx = self.client_transaction_tx.clone();

                self.rt.spawn(async move {
                    match tx.send(Err(code)).await {
                        Ok(()) => {}
                        Err(e) => {}
                    }
                });

                return;
            }
        }
    }

    fn on_transport_error(&self) {
        let client_transaction_tx = self.client_transaction_tx.clone();
        self.rt.spawn(async move {
            match client_transaction_tx.send(Err(0)).await {
                Ok(()) => {}
                Err(e) => {}
            }
        });
    }
}

pub struct MultiConferenceServiceV1Wrapper {
    pub service: Arc<MultiConferenceServiceV1>,
    pub tm: Arc<SipTransactionManager>,
}

struct MultiConferenceV1InviteResponseInner {
    pub status_code: u16,
    pub answer_sdp: String,
    pub event_listener: MultiConferenceEventListener,
    pub event_listener_context: MultiConferenceEventListenerContextWrapper,
}

pub struct MultiConferenceV1InviteResponseReceiver {
    tx: Option<oneshot::Sender<MultiConferenceV1InviteResponseInner>>,
}

impl MultiConferenceV1InviteResponseReceiver {
    pub fn accept_result(&mut self, resp: &MultiConferenceV1InviteResponse) {
        if let Some(tx) = self.tx.take() {
            if let Some(event_listener) = resp.event_listener {
                let answer_sdp = unsafe {
                    CStr::from_ptr(resp.answer_sdp)
                        .to_string_lossy()
                        .into_owned()
                };

                let event_listener_context = MultiConferenceEventListenerContextWrapper(
                    NonNull::new(resp.event_listener_context).unwrap(),
                );

                match tx.send(MultiConferenceV1InviteResponseInner {
                    status_code: resp.status_code,
                    answer_sdp,
                    event_listener,
                    event_listener_context,
                }) {
                    Ok(()) => {}
                    Err(e) => {}
                }
            }
        }
    }

    pub fn drop_result(&mut self) {
        self.tx.take();
    }
}

impl TransactionHandler for MultiConferenceServiceV1Wrapper {
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
                let mut is_v1_multi_conference_invite = false;

                if let Some(header) = header::search(req_headers, b"P-Asserted-Service", true) {
                    if header
                        .get_value()
                        .eq_ignore_ascii_case(b"urn:example:conference-service")
                    {
                        is_v1_multi_conference_invite = true;
                    }
                }

                if is_v1_multi_conference_invite {
                    if let Some(header) = header::search(req_headers, b"Contact", true) {
                        if let Some(conference_v1_contact) = header
                            .get_value()
                            .as_header_field()
                            .as_conference_v1_contact()
                        {
                            if let Some(header) = header::search(req_headers, b"Content-Type", true)
                            {
                                if let Some(content_type) =
                                    header.get_value().as_header_field().as_content_type()
                                {
                                    if content_type.major_type.eq_ignore_ascii_case(b"application")
                                        && content_type.sub_type.eq_ignore_ascii_case(b"sdp")
                                    {
                                        if let Body::Raw(raw) = req_body.as_ref() {
                                            if let Ok(sdp_string) = std::str::from_utf8(raw) {
                                                let offer_sdp = String::from(sdp_string);

                                                if let Some(resp_message) =
                                                    server_transaction::make_response(
                                                        message,
                                                        transaction.to_tag(),
                                                        180,
                                                        b"Ringing",
                                                    )
                                                {
                                                    if let Some((tx, mut rx)) = channels.take() {
                                                        let (d_tx, mut d_rx) = mpsc::channel(1);

                                                        let ongoing_dialogs_ =
                                                            Arc::clone(&ongoing_dialogs);

                                                        rt.spawn(async move {
                                                            if let Some(dialog) = d_rx.recv().await
                                                            {
                                                                ongoing_dialogs_
                                                                    .remove_dialog(&dialog);
                                                            }
                                                        });

                                                        if let Ok(dialog) =
                                                            SipDialog::try_new_as_uas(
                                                                message,
                                                                &resp_message,
                                                                move |d| match d_tx.blocking_send(d)
                                                                {
                                                                    Ok(()) => {}
                                                                    Err(e) => {}
                                                                },
                                                            )
                                                        {
                                                            let dialog = Arc::new(dialog);

                                                            ongoing_dialogs.add_dialog(&dialog);

                                                            let (conf_tx, conf_rx) =
                                                                mpsc::channel(8);

                                                            let session =
                                                                MultiConferenceV1Session {
                                                                    conference_id:
                                                                        conference_v1_contact
                                                                            .conference_uri,
                                                                    tx: conf_tx,
                                                                };

                                                            let session = Arc::new(session);

                                                            let (sess_tx, mut sess_rx) =
                                                                mpsc::channel(8);

                                                            let sip_session = SipSession::new(
                                                                &session,
                                                                SipSessionEventReceiver {
                                                                    tx: sess_tx,
                                                                    rt: Arc::clone(&rt),
                                                                },
                                                            );

                                                            let sip_session = Arc::new(sip_session);
                                                            let sip_session_ =
                                                                Arc::clone(&sip_session);

                                                            let registered_public_identity =
                                                                Arc::clone(
                                                                    &self
                                                                        .service
                                                                        .registered_public_identity,
                                                                );

                                                            let tm_ = Arc::clone(&self.tm);
                                                            let rt_ = Arc::clone(&rt);

                                                            rt.spawn(async move {
                                                                while let Some(ev) =
                                                                    sess_rx.recv().await
                                                                {
                                                                    match ev {
                                                                        SipSessionEvent::ShouldRefresh(dialog) => {
                                                                            if let Ok(mut message) =
                                                                                dialog.make_request(b"UPDATE", None)
                                                                            {
                                                                                message.add_header(Header::new(
                                                                                    b"Supported",
                                                                                    b"timer",
                                                                                ));

                                                                                let guard = registered_public_identity.lock().unwrap();

                                                                                if let Some((transport, _, _)) = &*guard
                                                                                {
                                                                                    tm_.send_request(
                                                                                            message,
                                                                                            transport,
                                                                                            UpdateMessageCallbacks {
                                                                                                // session_expires: None,
                                                                                                dialog,
                                                                                                sip_session: Arc::clone(
                                                                                                    &sip_session_,
                                                                                                ),
                                                                                                rt: Arc::clone(&rt_),
                                                                                            },
                                                                                            &rt_,
                                                                                        );
                                                                                }
                                                                            }
                                                                        }

                                                                        SipSessionEvent::Expired(dialog) => {
                                                                            if let Ok(message) =
                                                                                dialog.make_request(b"BYE", None)
                                                                            {
                                                                                let guard = registered_public_identity.lock().unwrap();

                                                                                if let Some((transport, _, _)) = &*guard
                                                                                {
                                                                                    tm_.send_request(
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

                                                            let event_listener =
                                                                Arc::new(Mutex::new(None));
                                                            let event_listener_ =
                                                                Arc::clone(&event_listener);

                                                            sip_session.setup_early_dialog(&dialog, MultiConferenceV1SessionDialogEventReceiver {
                                                                sip_session: Arc::clone(&sip_session),
                                                                event_listener,
                                                                rt: Arc::clone(&rt),
                                                            });

                                                            let inner = MultiConferenceV1Internal {
                                                                sip_session,
                                                            };
                                                            let inner = Arc::new(inner);

                                                            let conference_v1 =
                                                                MultiConferenceV1 { inner };

                                                            let (handler_tx, handler_rx) =
                                                                oneshot::channel();

                                                            let response_receiver = MultiConferenceV1InviteResponseReceiver{
                                                                tx: Some(handler_tx),
                                                            };

                                                            (self
                                                                .service
                                                                .multi_conference_invite_handler)(
                                                                conference_v1,
                                                                offer_sdp,
                                                                response_receiver,
                                                            );

                                                            let (tx_, mut rx_) = mpsc::channel::<
                                                                ServerTransactionEvent,
                                                            >(
                                                                8
                                                            );

                                                            rt.spawn(async move {
                                                                while let Some(ev) = rx.recv().await {
                                                                    match ev {
                                                                        ServerTransactionEvent::Cancelled => {
                                                                            todo!()
                                                                        }
                                                                        _ => {}
                                                                    }

                                                                    match tx_.send(ev).await {
                                                                        Ok(()) => {}
                                                                        Err(e) => {}
                                                                    }
                                                                }
                                                            });

                                                            let registered_public_identity =
                                                                Arc::clone(
                                                                    &self
                                                                        .service
                                                                        .registered_public_identity,
                                                                );

                                                            let transaction_ =
                                                                Arc::clone(transaction);

                                                            let rt_ = Arc::clone(&rt);

                                                            rt.spawn(async move {

                                                                let message = transaction_.message();

                                                                match handler_rx.await {
                                                                    Ok(resp) => {

                                                                        if resp.status_code >= 200 && resp.status_code < 300 {

                                                                            if let Some(mut resp_message) = server_transaction::make_response(message, transaction_.to_tag(), 200, b"OK") {

                                                                                {
                                                                                    let guard = registered_public_identity.lock().unwrap();

                                                                                    if let Some((
                                                                                        transport,
                                                                                        contact_identity,
                                                                                        instance_id,
                                                                                    )) = &*guard {
                                                                                        let transport_ = transaction_.transport();
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

                                                                                let sdp_body = resp.answer_sdp.as_bytes().to_vec();
                                                                                let sdp_body = Body::Raw(sdp_body);
                                                                                let sdp_body = Arc::new(sdp_body);
                                                                                let sdp_body_len = resp.answer_sdp.len();

                                                                                resp_message.add_header(Header::new(b"Content-Type", b"application/json"));

                                                                                resp_message.add_header(Header::new(b"Content-Length", format!("{}", sdp_body_len)));

                                                                                resp_message.set_body(sdp_body);

                                                                                let mut guard = event_listener_.lock().unwrap();

                                                                                *guard = Some((conf_rx, resp.event_listener, resp.event_listener_context));

                                                                                rt_.spawn(async move {

                                                                                    while let Some(ev) = rx_.recv().await {
                                                                                        match ev {
                                                                                            ServerTransactionEvent::Cancelled => {
                                                                                                todo!()
                                                                                            },
                                                                                            _ => {}
                                                                                        }
                                                                                    }
                                                                                });

                                                                                dialog.confirm();

                                                                                server_transaction::send_response(
                                                                                    transaction_,
                                                                                    resp_message,
                                                                                    tx,
                                                                                    // &timer,
                                                                                    &rt_,
                                                                                );

                                                                                return ;
                                                                            }
                                                                        }
                                                                    }

                                                                    Err(e) => {

                                                                    }
                                                                }

                                                                if let Some(resp_message) = server_transaction::make_response(message, transaction_.to_tag(), 486, b"Busy Here") {
                                                                    server_transaction::send_response(
                                                                        transaction_,
                                                                        resp_message,
                                                                        tx,
                                                                        // &timer,
                                                                        &rt_,
                                                                    );
                                                                }
                                                            });

                                                            return true;
                                                        }

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
                                }
                            }

                            if let Some(resp_message) = server_transaction::make_response(
                                message,
                                transaction.to_tag(),
                                400,
                                b"Bad Request",
                            ) {
                                if let Some((tx, mut rx)) = channels.take() {
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
            }
        }

        false
    }
}
