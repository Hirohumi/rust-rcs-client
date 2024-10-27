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
    ops::Add,
    sync::{Arc, Mutex, MutexGuard},
    time::Duration,
};

use tokio::{
    runtime::Runtime,
    select,
    sync::oneshot,
    time::{interval, Instant},
};

use rust_rcs_core::{
    ffi::log::platform_log,
    internet::{name_addr::AsNameAddr, Body, Header},
    sip::{
        sip_subscription::{
            subscribe_request::SubscribeRequest,
            subscriber::{Subscriber, SubscriberEvent},
            InitialSubscribeContext, SubscriptionManager,
        },
        SipCore, SipMessage, SipTransactionManager, SipTransport, SUBSCRIBE,
    },
    util::{rand, raw_string::StrEq},
};

use uuid::Uuid;

use super::{
    reg_info_xml::{parse_xml, ContactNode, RegistrationNode},
    registration::Registration,
};

const LOG_TAG: &str = "librust_rcs_client";

#[derive(Debug)]
pub struct AddressOfRecord {
    _is_implicit_address: bool,
    record_id: String,
    _public_user_id: String,
}

#[derive(Debug)]
pub struct FlowInfo {
    uri: String,
    flow_id: String,
    expires: u32,
    registerred_addresses: Vec<AddressOfRecord>,
}

pub struct FlowManager {
    managed_flow: Arc<
        Mutex<(
            Option<(
                Arc<SipTransport>,
                String,
                Arc<Registration>,
                oneshot::Sender<()>,
            )>,
            Option<(String, Arc<Subscriber>)>,
            Vec<FlowInfo>,
        )>,
    >,

    sm: Arc<SubscriptionManager>,
    tm: Arc<SipTransactionManager>,

    state_callback:
        Box<dyn Fn(bool, Arc<SipTransport>, Option<String>, Arc<Registration>) + Send + Sync>,
}

impl FlowManager {
    pub fn new<CB>(
        sm: &Arc<SubscriptionManager>,
        tm: &Arc<SipTransactionManager>,
        state_callback: CB,
    ) -> FlowManager
    where
        CB: Fn(bool, Arc<SipTransport>, Option<String>, Arc<Registration>) + Send + Sync + 'static,
    {
        FlowManager {
            // managed_reg: Arc::new(Mutex::new(None)),
            // managed_sub: Arc::new(Mutex::new(None)),
            managed_flow: Arc::new(Mutex::new((None, None, Vec::new()))),

            sm: Arc::clone(sm),
            tm: Arc::clone(tm),

            state_callback: Box::new(state_callback),
        }
    }

    pub fn observe_registration(
        &self,
        transport: &Arc<SipTransport>,
        transport_address: String,
        registration: &Arc<Registration>,
        core: &Arc<SipCore>,
        rt: &Arc<Runtime>,
    ) {
        let (tx, mut rx) = oneshot::channel();

        let mut guard = self.managed_flow.lock().unwrap();

        guard.0 = Some((
            Arc::clone(transport),
            transport_address,
            Arc::clone(registration),
            tx,
        ));

        if let Some((_, subscriber)) = &guard.1 {
            subscriber.stop_subscribing(&self.sm, &self.tm, transport, rt);

            guard.1.take();
            guard.2.clear();
        }

        let transport_ = Arc::clone(transport);

        let core_ = Arc::clone(core);
        let rt_ = Arc::clone(rt);

        rt.spawn(async move {
            let mut interval = interval(Duration::from_secs(120)); // to-do: make it adjustable
            let mut ticks = 0;

            loop {
                select! {
                    _ = &mut rx => {
                        break
                    },

                    _ = interval.tick() => {
                        if ticks > 0 {
                            let tm = core_.get_transaction_manager();
                            tm.send_heartbeat(&transport_, &rt_);
                        }
                        ticks += 1;
                    },
                };
            }
        });
    }

    pub fn schedule_next_subscribe_directly(
        &self,
        transport: &Arc<SipTransport>,
        core: &Arc<SipCore>,
        rt: &Arc<Runtime>,
    ) {
        let sm = core.get_subscription_manager();
        let tm = core.get_transaction_manager();

        let guard = self.managed_flow.lock().unwrap();

        match &guard.1 {
            Some((_, subscriber)) => {
                subscriber.extend_all_subscriptions(3600, &sm, &tm, transport, rt);
            }
            None => {}
        }
    }

    pub fn stop_observation(&self, transport: &Arc<SipTransport>) -> Option<Arc<Registration>> {
        let mut guard = self.managed_flow.lock().unwrap();
        if let Some((transport_, transport_address, registration, tx)) = guard.0.take() {
            if Arc::ptr_eq(transport, &transport_) {
                let _ = tx.send(()); // stop heart-beat

                guard.1.take();
                guard.2.clear();

                return Some(registration);
            } else {
                guard
                    .0
                    .replace((transport_, transport_address, registration, tx));
            }
        }

        None
    }
}

pub enum SubscriptionEvent {
    ExpirationRefreshed(Instant),
}

pub fn on_registration_authenticated<CB>(
    flow_manager: &Arc<FlowManager>,
    impu: String,
    transport: &Arc<SipTransport>,
    transport_address: String, // use this in Contact Header ?
    registration: &Arc<Registration>,
    sip_instance_id: Uuid,
    core: &Arc<SipCore>,
    rt: &Arc<Runtime>,
    state_callback: CB,
) where
    CB: Fn(SubscriptionEvent) + Send + Sync + 'static,
{
    platform_log(LOG_TAG, "on_registration_authenticated()");

    let mut guard = flow_manager.managed_flow.lock().unwrap();

    match &guard.0 {
        Some((_, _, registration_, _)) => {
            if !Arc::ptr_eq(registration, registration_) {
                return;
            }
        }

        None => {}
    }

    let flow_manager_ = Arc::clone(&flow_manager);

    let impu_ = impu.clone();

    let transport_ = Arc::clone(transport);
    let registration_ = Arc::clone(registration);

    let transport_type = transport_.get_contact_transport_type();

    let subscriber = Subscriber::new(
        "Registrar",
        // only single source dialog allowed for reg-info subscription
        move |event| match event {
            SubscriberEvent::ReceivedNotify(_, content_type, body) => {
                platform_log(LOG_TAG, "on NOTIFY content");
                if content_type.equals_bytes(b"application/reginfo+xml", true) {
                    if let Body::Raw(r) = body.as_ref() {
                        if let Some(reg_info) = parse_xml(r) {
                            platform_log(LOG_TAG, "reginfo+xml parsed successfully");
                            let mut guard = flow_manager_.managed_flow.lock().unwrap();

                            for registration_node in &reg_info.registration_nodes {
                                for contact_node in &registration_node.contact_nodes {
                                    if &contact_node.state == "terminated"
                                        && (&contact_node.event == "unregistered"
                                            || &contact_node.event == "rejected"
                                            || &contact_node.event == "deactivated")
                                    {
                                        remove_aor_within_flow(
                                            contact_node,
                                            registration_node,
                                            &mut guard,
                                        );
                                    } else if &contact_node.state == "active" {
                                        if &contact_node.event == "created"
                                            || &contact_node.event == "registered"
                                        {
                                            create_aor_within_flow(
                                                contact_node,
                                                registration_node,
                                                &mut guard,
                                            );
                                        } else if &contact_node.event == "shortened"
                                            || &contact_node.event == "refreshed"
                                        {
                                            match update_aor_within_flow(contact_node, &mut guard) {
                                                Ok(()) => {}

                                                Err(()) => {
                                                    guard.1.take();
                                                    guard.2.clear();

                                                    if let Some((transport, _, registration, _)) =
                                                        &guard.0
                                                    {
                                                        (flow_manager_.state_callback)(
                                                            false,
                                                            Arc::clone(transport),
                                                            None,
                                                            Arc::clone(registration),
                                                        );
                                                    }

                                                    return;
                                                }
                                            }
                                        }
                                    }
                                }
                            }

                            match &guard.0 {
                                Some((_, t_addr, _, _)) => {
                                    match &guard.1 {
                                        Some((impu, _)) => {
                                            let uri = if let Some(idx) = impu.find('@') {
                                                // we should really be more specific about the kind of IMPU that we use to identify our selves
                                                format!("{}@{}", &impu[0..idx], t_addr)
                                            } else {
                                                format!("{}@{}", impu, t_addr)
                                            };

                                            platform_log(
                                                LOG_TAG,
                                                format!("registration is bound at {}", &uri),
                                            );

                                            for flow in &*guard.2 {
                                                platform_log(
                                                    LOG_TAG,
                                                    format!("active flow {:?}", flow),
                                                );

                                                let mut is_current_registry = false;

                                                if uri.eq_ignore_ascii_case(&flow.uri) {
                                                    is_current_registry = true;
                                                }

                                                if !is_current_registry {
                                                    if let Some(flow_uri) = flow
                                                        .uri
                                                        .as_bytes()
                                                        .as_name_addresses()
                                                        .first()
                                                    {
                                                        if let Some(uri_part) = &flow_uri.uri_part {
                                                            if uri
                                                                .as_bytes()
                                                                .equals_bytes(uri_part.uri, true)
                                                            {
                                                                is_current_registry = true;
                                                            }
                                                        }
                                                    }
                                                }

                                                if is_current_registry {
                                                    let public_user_identity = impu.clone();
                                                    (flow_manager_.state_callback)(
                                                        true,
                                                        Arc::clone(&transport_),
                                                        Some(public_user_identity),
                                                        Arc::clone(&registration_),
                                                    );

                                                    let expiration = Instant::now().add(
                                                        Duration::from_secs(flow.expires as u64),
                                                    );

                                                    state_callback(
                                                        SubscriptionEvent::ExpirationRefreshed(
                                                            expiration,
                                                        ),
                                                    );

                                                    return;
                                                }
                                            }
                                        }
                                        None => {}
                                    }
                                }
                                None => {}
                            }
                        }
                    }
                }

                let mut guard = flow_manager_.managed_flow.lock().unwrap();

                guard.1.take();
                guard.2.clear();

                if let Some((transport, _, registration, _)) = &guard.0 {
                    (flow_manager_.state_callback)(
                        false,
                        Arc::clone(transport),
                        None,
                        Arc::clone(registration),
                    );
                }
            }

            SubscriberEvent::NearExpiration(_) => {
                platform_log(LOG_TAG, "on NearExpiration event");
            }

            SubscriberEvent::Terminated(_, can_resubscribe, retry_after) => {
                platform_log(
                    LOG_TAG,
                    format!("on Terminated event: {}, {}", can_resubscribe, retry_after),
                );
                // only one subscription is allowed so terminating one means terminating all
                let mut guard = flow_manager_.managed_flow.lock().unwrap();

                guard.1.take();
                guard.2.clear();

                if let Some((transport, _, registration, _)) = &guard.0 {
                    (flow_manager_.state_callback)(
                        false,
                        Arc::clone(transport),
                        None,
                        Arc::clone(registration),
                    );
                }
            }

            SubscriberEvent::SubscribeFailed(code) => {
                platform_log(LOG_TAG, format!("on SubscribeFailed event: {}", code));
                if code == 200 {
                    let guard = flow_manager_.managed_flow.lock().unwrap();

                    match &guard.0 {
                        Some((_, _, _, _)) => match &guard.1 {
                            Some((impu, _)) => {
                                if let Some((transport, _, registration, _)) = &guard.0 {
                                    let public_user_identity = impu.clone();
                                    (flow_manager_.state_callback)(
                                        true,
                                        Arc::clone(transport),
                                        Some(public_user_identity),
                                        Arc::clone(registration),
                                    );
                                }

                                return;
                            }
                            None => {}
                        },
                        None => {}
                    }
                }

                let mut guard = flow_manager_.managed_flow.lock().unwrap();

                guard.1.take();
                guard.2.clear();

                if let Some((transport, _, registration, _)) = &guard.0 {
                    (flow_manager_.state_callback)(
                        false,
                        Arc::clone(transport),
                        None,
                        Arc::clone(registration),
                    );
                }
            }
        },
        move |dialog, expiration| -> Result<SipMessage, ()> {
            let request_message = match dialog {
                Some(dialog) => match dialog.make_request(SUBSCRIBE, None) {
                    Ok(message) => Ok(message),
                    Err(e) => {
                        platform_log(LOG_TAG, format!("sip in-dialog message build error {}", e));
                        Err(())
                    }
                },

                None => {
                    let mut message = SipMessage::new_request(SUBSCRIBE, impu_.as_bytes());

                    message.add_header(Header::new(
                        b"Call-ID",
                        String::from(
                            Uuid::new_v4()
                                .as_hyphenated()
                                .encode_lower(&mut Uuid::encode_buffer()),
                        ),
                    ));

                    message.add_header(Header::new(b"CSeq", b"1 SUBSCRIBE"));

                    let tag = rand::create_raw_alpha_numeric_string(8);
                    let tag = String::from_utf8_lossy(&tag);

                    message.add_header(Header::new(b"From", format!("<{}>;tag={}", impu_, tag)));

                    message.add_header(Header::new(b"To", format!("<{}>", impu_)));

                    let contact;
                    if let Some(idx) = impu_.find('@') {
                        contact = &(impu_[0..idx]);
                    } else {
                        contact = &impu_;
                    }

                    message.add_header(Header::new(
                        b"Contact",
                        format!(
                            "<{}@{};transport={}>;+sip.instance=\"<urn:uuid:{}>\"",
                            contact,
                            transport_address,
                            transport_type,
                            sip_instance_id
                                .hyphenated()
                                .encode_lower(&mut Uuid::encode_buffer())
                        ),
                    )); // to-do: is transport_address neccessary? also check NOTIFY 200 Ok

                    message.add_header(Header::new(b"Expires", expiration.to_string()));

                    message.add_header(Header::new(b"Event", b"reg"));

                    message.add_header(Header::new(b"Accept-Contact", b"*"));

                    message.add_header(Header::new(b"Accept", b"application/reginfo+xml"));

                    message.add_header(Header::new(b"Accept-Encoding", b"gzip"));

                    Ok(message)
                }
            };

            request_message
        },
    );

    let subscriber = Arc::new(subscriber);
    let subscriber_ = Arc::clone(&subscriber);

    if let Ok(request_message) = subscriber.build_request(None, 3600) {
        if let Ok(subscribe_request) = SubscribeRequest::new(&request_message, subscriber) {
            let subscribe_request = Arc::new(subscribe_request);

            core.get_subscription_manager()
                .register_request(Arc::clone(&subscribe_request), rt);

            // let ongoing_dialogs = core.get_ongoing_dialogs();
            let core_ = Arc::clone(&core);
            let rt_ = Arc::clone(rt);

            core.get_transaction_manager().send_request(
                request_message,
                &transport,
                InitialSubscribeContext::new(subscribe_request, core_, rt_),
                rt,
            )
        }
    }

    guard.1 = Some((impu, subscriber_));
}

fn create_aor(
    registerred_addresses: &mut Vec<AddressOfRecord>,
    registration_node: &RegistrationNode,
    is_implicit_address: bool,
) {
    for aor in &*registerred_addresses {
        if aor.record_id == registration_node.id {
            return;
        }
    }

    registerred_addresses.push(AddressOfRecord {
        _is_implicit_address: is_implicit_address,
        record_id: registration_node.id.clone(),
        _public_user_id: registration_node.aor.clone(),
    });
}

fn create_aor_within_flow(
    contact_node: &ContactNode,
    registration_node: &RegistrationNode,
    guard: &mut MutexGuard<(
        Option<(
            Arc<SipTransport>,
            String,
            Arc<Registration>,
            oneshot::Sender<()>,
        )>,
        Option<(String, Arc<Subscriber>)>,
        Vec<FlowInfo>,
    )>,
) {
    platform_log(LOG_TAG, "create_aor_within_flow");

    let is_implicit_address = if &contact_node.event == "created" {
        true
    } else {
        false
    };

    for flow in &mut guard.2 {
        if flow.flow_id == contact_node.id {
            create_aor(
                &mut flow.registerred_addresses,
                registration_node,
                is_implicit_address,
            );
            return;
        }
    }

    let mut registerred_addresses = Vec::new();

    create_aor(
        &mut registerred_addresses,
        registration_node,
        is_implicit_address,
    );

    guard.2.push(FlowInfo {
        uri: contact_node.uri.clone(),
        flow_id: contact_node.id.clone(),
        expires: match &contact_node.expires {
            Some(expires) => match u32::from_str_radix(expires, 10) {
                Ok(expires) => expires,
                _ => 0,
            },
            None => 0,
        },
        registerred_addresses,
    });
}

fn update_aor_within_flow(
    contact_node: &ContactNode,
    guard: &mut MutexGuard<(
        Option<(
            Arc<SipTransport>,
            String,
            Arc<Registration>,
            oneshot::Sender<()>,
        )>,
        Option<(String, Arc<Subscriber>)>,
        Vec<FlowInfo>,
    )>,
) -> Result<(), ()> {
    platform_log(LOG_TAG, "update_aor_within_flow");

    for flow in &mut guard.2 {
        if flow.flow_id == contact_node.id {
            flow.expires = match &contact_node.expires {
                Some(expires) => match u32::from_str_radix(expires, 10) {
                    Ok(expires) => expires,
                    _ => 0,
                },
                None => 0,
            };
            return Ok(());
        }
    }

    Err(())
}

fn remove_aor_within_flow(
    contact_node: &ContactNode,
    registration_node: &RegistrationNode,
    guard: &mut MutexGuard<(
        Option<(
            Arc<SipTransport>,
            String,
            Arc<Registration>,
            oneshot::Sender<()>,
        )>,
        Option<(String, Arc<Subscriber>)>,
        Vec<FlowInfo>,
    )>,
) {
    platform_log(LOG_TAG, "remove_aor_within_flow");

    let mut i = 0;
    for flow in &mut guard.2 {
        if flow.flow_id == contact_node.id {
            let mut j = 0;
            for aor in &mut flow.registerred_addresses {
                if aor.record_id == registration_node.id {
                    flow.registerred_addresses.swap_remove(j);
                    break;
                }
                j += 1;
            }
            if flow.registerred_addresses.is_empty() {
                guard.2.swap_remove(i);
            }
            break;
        }
        i += 1;
    }
}
