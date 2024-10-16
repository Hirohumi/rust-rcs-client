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

use base64::{engine::general_purpose, Engine};
use tokio::{
    runtime::Runtime,
    time::{sleep_until, Instant},
};

use uuid::Uuid;

use rust_rcs_core::{
    ffi::log::platform_log,
    internet::{
        header::{search, HeaderSearch},
        name_addr::AsNameAddr,
        AsHeaderField, Header,
    },
    security::{
        aka::{aka_do_challenge, AkaChallenge, AkaResponse, AsAkaAlgorithm},
        authentication::{
            challenge::ChallengeParser,
            digest::{DigestAnswerParams, DigestChallengeParams, DigestCredentials},
        },
    },
    sip::{ClientTransactionCallbacks, SipCore, SipMessage, SipTransport, REGISTER},
    third_gen_pp::addressing,
    util::{
        rand,
        raw_string::{FromRawStr, StrEq, ToInt},
    },
};

use super::{
    authentication_type::AuthenticationType,
    flow::{on_registration_authenticated, FlowManager, SubscriptionEvent},
};

const LOG_TAG: &str = "librust_rcs_client";

pub enum RegistrationState {
    IDLE,
    REQUESTING,
    REGISTERED(String, Instant), // (un-barred IMPU, expiration)
    RELEASED,
}

pub enum RegistrationEvent {
    Registered(String),
    Refreshed,
    Released,
}

pub struct Registration {
    reg_state: Arc<Mutex<(u32, RegistrationState, DigestAnswerParams)>>, // (seq, state, params)

    transport: Arc<SipTransport>,
    transport_address: String,

    registration_id: u32,

    sip_instance_id: Uuid,

    subscription_id: i32,

    impi: String,
    impu: String,

    home_domain: String,

    authentication_type: AuthenticationType,

    call_id: Uuid,
}

impl Registration {
    pub fn new(
        subscription_id: i32,
        impi: String,
        impu: String,
        home_domain: String,
        authentication_type: AuthenticationType,
        transport: Arc<SipTransport>,
        transport_address: String,
        registration_id: u32,
        sip_instance_id: Uuid,
    ) -> Registration {
        let authorization_params = DigestAnswerParams {
            realm: match &authentication_type {
                AuthenticationType::Aka(_, realm, _) => realm.as_bytes().to_vec(),

                AuthenticationType::Digest(_, realm, _, _) => realm.as_bytes().to_vec(),
            },

            algorithm: match &authentication_type {
                AuthenticationType::Aka(algorithm, _, _) => Some(algorithm.as_bytes().to_vec()),

                AuthenticationType::Digest(algorithm, _, _, _) => {
                    Some(algorithm.as_bytes().to_vec())
                }
            },

            username: match &authentication_type {
                AuthenticationType::Aka(_, _, username) => username.as_bytes().to_vec(),

                AuthenticationType::Digest(_, _, username, _) => username.as_bytes().to_vec(),
            },

            uri: format!("sip:{}", home_domain).into_bytes(),

            challenge: None,
            credentials: None,
        };

        Registration {
            reg_state: Arc::new(Mutex::new((
                0,
                RegistrationState::IDLE,
                authorization_params,
            ))),

            transport,
            transport_address,

            registration_id,

            sip_instance_id,

            subscription_id,

            impi,
            impu,

            home_domain,

            authentication_type,

            call_id: Uuid::new_v4(),
        }
    }
}

pub fn start_register<CB>(
    registration: &Arc<Registration>,
    flow_manager: &Arc<FlowManager>,
    core: &Arc<SipCore>,
    rt: &Arc<Runtime>,
    state_callback: CB,
) where
    CB: Fn(RegistrationEvent) + Send + Sync + 'static,
{
    platform_log(LOG_TAG, "calling registration->start_register()");

    let mut guard = registration.reg_state.lock().unwrap();

    match &guard.1 {
        RegistrationState::IDLE => {
            guard.1 = RegistrationState::REQUESTING;
        }
        RegistrationState::REQUESTING => return,
        RegistrationState::REGISTERED(_, _) => return,
        RegistrationState::RELEASED => return,
    }

    let req_message = make_request(
        &registration.home_domain,
        &registration.impu,
        registration.call_id,
        &registration.transport_address,
        registration.registration_id,
        registration.sip_instance_id,
        &registration.authentication_type,
        &mut guard,
        false,
    );

    core.get_transaction_manager().send_request(
        req_message,
        &registration.transport,
        RegisterContext {
            failure_count: 0,
            registration: Arc::clone(registration),
            flow_manager: Arc::clone(flow_manager),
            core: Arc::clone(core),
            rt: Arc::clone(rt),
            state_callback: Arc::new(Box::new(state_callback)),
        },
        rt,
    );
}

struct RegisterContext {
    failure_count: i32,
    registration: Arc<Registration>,
    flow_manager: Arc<FlowManager>,
    core: Arc<SipCore>,
    rt: Arc<Runtime>,
    state_callback: Arc<Box<dyn Fn(RegistrationEvent) + Send + Sync>>,
}

impl ClientTransactionCallbacks for RegisterContext {
    fn on_provisional_response(&self, _message: SipMessage) {}

    fn on_final_response(&self, message: SipMessage) {
        if let SipMessage::Response(l, headers, _) = &message {
            platform_log(
                LOG_TAG,
                format!("registration->on_final_response(): {}", l.status_code),
            );

            if l.status_code >= 200 && l.status_code < 300 {
                let mut associated_uris = Vec::new();
                let mut default_public_identity: Option<String> = None; // to-do: we should also have a preferred IMPU (MSISDN) to use in P-Preferred-Identity header
                if let Some(headers) = headers {
                    for associated_uri_header in
                        HeaderSearch::new(headers, b"P-Associated-URI", true)
                    {
                        for associated_uri in associated_uri_header.get_value().as_name_addresses()
                        {
                            if let Some(uri_part) = associated_uri.uri_part {
                                if let Ok(s) = std::str::from_utf8(uri_part.uri) {
                                    platform_log(LOG_TAG, format!("unbarred IMPU {}", s));

                                    associated_uris.push(String::from(s));
                                    if uri_part.uri.starts_with(b"sip:") {
                                        if default_public_identity.is_none() {
                                            default_public_identity.replace(String::from(s));
                                        }
                                    }
                                }
                            }
                        }
                    }
                }

                let unbarred_impu: String;
                if let Some(default_public_identity) = default_public_identity.take() {
                    unbarred_impu = default_public_identity;
                } else {
                    unbarred_impu = addressing::tmpu_from_impi(&self.registration.impi);
                }

                platform_log(
                    LOG_TAG,
                    format!(
                        "register success with first unbarred IMPU {}",
                        &unbarred_impu
                    ),
                );

                let mut expires: Option<u32> = None;
                if let Some(headers) = headers {
                    if let Some(e) = get_contact_expires(headers) {
                        expires.replace(e);
                    } else if let Some(e) = get_sip_expires(headers) {
                        expires.replace(e);
                    }
                }

                if let Some(expires) = expires {
                    let expiration = Instant::now().add(Duration::from_secs(expires as u64));

                    {
                        let mut guard = self.registration.reg_state.lock().unwrap();

                        match &guard.1 {
                            RegistrationState::REQUESTING => {
                                guard.1 = RegistrationState::REGISTERED(
                                    unbarred_impu.clone(),
                                    expiration,
                                );
                                (self.state_callback)(RegistrationEvent::Registered(
                                    unbarred_impu.clone(),
                                ));

                                let registration_ = Arc::clone(&self.registration);

                                let transport = Arc::clone(&self.registration.transport);
                                let transport_address = self.registration.transport_address.clone();

                                let flow_manager_ = Arc::clone(&self.flow_manager);

                                let core_ = Arc::clone(&self.core);
                                let rt_ = Arc::clone(&self.rt);

                                let state_callback_ = Arc::clone(&self.state_callback);

                                on_registration_authenticated(
                                    &self.flow_manager,
                                    unbarred_impu,
                                    &transport,
                                    transport_address,
                                    &self.registration,
                                    self.registration.sip_instance_id,
                                    &self.core,
                                    &self.rt,
                                    move |ev| match ev {
                                        SubscriptionEvent::ExpirationRefreshed(expiration) => {
                                            schedule_refresh(
                                                &registration_,
                                                expiration,
                                                &flow_manager_,
                                                &core_,
                                                &rt_,
                                                &state_callback_,
                                            );
                                        }
                                    },
                                );
                            }
                            _ => {}
                        }
                    }

                    schedule_refresh(
                        &self.registration,
                        expiration,
                        &self.flow_manager,
                        &self.core,
                        &self.rt,
                        &self.state_callback,
                    );

                    return;
                } else {
                    // to-do: might want to de-register
                }
            } else if l.status_code == 401 {
                if self.failure_count > 0 {
                    {
                        let mut guard = self.registration.reg_state.lock().unwrap();
                        guard.1 = RegistrationState::RELEASED;
                    }

                    (self.state_callback)(RegistrationEvent::Released);
                    return;
                }

                let mut guard = self.registration.reg_state.lock().unwrap();

                guard.2.challenge.take();
                guard.2.credentials.take();

                if let Some(headers) = headers {
                    if let Some(www_authenticate_header) =
                        search(headers, b"WWW-Authenticate", true)
                    {
                        let www_authenticate_header_value = www_authenticate_header.get_value();
                        platform_log(
                            LOG_TAG,
                            format!(
                                "on_final_response with WWW-Authenticate: {}",
                                String::from_utf8_lossy(www_authenticate_header_value)
                            ),
                        );
                        let parser = ChallengeParser::new(www_authenticate_header_value);

                        for challenge in parser {
                            if let Some(params) = DigestChallengeParams::from_challenge(challenge) {
                                let digest_challenge = params.to_challenge();

                                guard.2.challenge.replace(digest_challenge);

                                break;
                            }
                        }
                    }
                }

                if let Some(challenge) = &guard.2.challenge {
                    match &self.registration.authentication_type {
                        AuthenticationType::Aka(_, _, _) => {
                            if let Ok(_) = challenge.algorithm.as_aka_algorithm() {
                                if let Ok(aka_challenge) =
                                    AkaChallenge::from_raw_str(&challenge.nonce)
                                {
                                    if let Ok(aka_response) = aka_do_challenge(
                                        &aka_challenge,
                                        self.registration.subscription_id,
                                    ) {
                                        match aka_response {
                                            AkaResponse::Successful(res, _k) => {
                                                let client_nonce =
                                                    rand::create_raw_alpha_numeric_string(8);

                                                guard.2.credentials.replace(DigestCredentials {
                                                    password: res,
                                                    client_data: Some(b"REGISTER".to_vec()),
                                                    client_nonce: Some((client_nonce, 0)),
                                                    entity_digest: None,
                                                    extra_params: Vec::new(),
                                                });
                                            }

                                            AkaResponse::SyncFailure(auts) => {
                                                let mut extra_params = Vec::new();

                                                let encoded =
                                                    general_purpose::STANDARD.encode(&auts);
                                                let encoded = Vec::from(encoded);

                                                extra_params.push((b"auts".to_vec(), encoded));

                                                let client_nonce =
                                                    rand::create_raw_alpha_numeric_string(8);

                                                guard.2.credentials.replace(DigestCredentials {
                                                    password: Vec::new(),
                                                    client_data: Some(b"REGISTER".to_vec()),
                                                    client_nonce: Some((client_nonce, 0)),
                                                    entity_digest: None,
                                                    extra_params,
                                                });
                                            }
                                        }

                                        let registration_ = Arc::clone(&self.registration);

                                        let flow_manager_ = Arc::clone(&self.flow_manager);

                                        let core_ = Arc::clone(&self.core);
                                        let rt_ = Arc::clone(&self.rt);

                                        let state_callback_ = Arc::clone(&self.state_callback);

                                        let req_message = make_request(
                                            &self.registration.home_domain,
                                            &self.registration.impu,
                                            self.registration.call_id,
                                            &self.registration.transport_address,
                                            self.registration.registration_id,
                                            self.registration.sip_instance_id,
                                            &self.registration.authentication_type,
                                            &mut guard,
                                            false,
                                        );

                                        self.core.get_transaction_manager().send_request(
                                            req_message,
                                            &self.registration.transport,
                                            RegisterContext {
                                                failure_count: 1,
                                                registration: registration_,
                                                flow_manager: flow_manager_,
                                                core: core_,
                                                rt: rt_,
                                                state_callback: state_callback_,
                                            },
                                            &self.rt,
                                        );

                                        return;
                                    }
                                }
                            }
                        }

                        AuthenticationType::Digest(_, _, _, password) => {
                            let client_nonce = rand::create_raw_alpha_numeric_string(8);

                            guard.2.credentials.replace(DigestCredentials {
                                password: password.as_bytes().to_vec(),
                                client_data: Some(b"REGISTER".to_vec()),
                                client_nonce: Some((client_nonce, 0)),
                                entity_digest: None,
                                extra_params: Vec::new(),
                            });

                            let registration_ = Arc::clone(&self.registration);

                            let flow_manager_ = Arc::clone(&self.flow_manager);

                            let core_ = Arc::clone(&self.core);
                            let rt_ = Arc::clone(&self.rt);

                            let state_callback_ = Arc::clone(&self.state_callback);

                            let req_message = make_request(
                                &self.registration.home_domain,
                                &self.registration.impu,
                                self.registration.call_id,
                                &self.registration.transport_address,
                                self.registration.registration_id,
                                self.registration.sip_instance_id,
                                &self.registration.authentication_type,
                                &mut guard,
                                false,
                            );

                            self.core.get_transaction_manager().send_request(
                                req_message,
                                &self.registration.transport,
                                RegisterContext {
                                    failure_count: 1,
                                    registration: registration_,
                                    flow_manager: flow_manager_,
                                    core: core_,
                                    rt: rt_,
                                    state_callback: state_callback_,
                                },
                                &self.rt,
                            );

                            return;
                        }
                    }
                }
            }

            {
                let mut guard = self.registration.reg_state.lock().unwrap();
                guard.1 = RegistrationState::RELEASED;
            }

            (self.state_callback)(RegistrationEvent::Released)
        }
    }

    fn on_transport_error(&self) {
        {
            let mut guard = self.registration.reg_state.lock().unwrap();
            guard.1 = RegistrationState::RELEASED;
        }

        (self.state_callback)(RegistrationEvent::Released)
    }
}

fn get_contact_expires(headers: &Vec<Header>) -> Option<u32> {
    if let Some(contact_header) = search(headers, b"Contact", true) {
        for parameter in contact_header
            .get_value()
            .as_header_field()
            .get_parameter_iterator()
        {
            if parameter.name.equals_bytes(b"expires", true) {
                if let Some(expires) = parameter.value {
                    if let Ok(expires) = expires.to_int::<u32>() {
                        return Some(expires);
                    }
                }
            }
        }
    }

    None
}

fn get_sip_expires(headers: &Vec<Header>) -> Option<u32> {
    if let Some(expires_header) = search(headers, b"Expires", true) {
        if let Ok(expires) = expires_header.get_value().to_int::<u32>() {
            return Some(expires);
        }
    }

    None
}

pub fn schedule_refresh(
    registration: &Arc<Registration>,
    expiration: Instant,
    flow_manager: &Arc<FlowManager>,
    core: &Arc<SipCore>,
    rt: &Arc<Runtime>,
    state_callback: &Arc<Box<dyn Fn(RegistrationEvent) + Send + Sync + 'static>>,
) {
    platform_log(LOG_TAG, "calling schedule_refresh()");

    {
        let guard = registration.reg_state.lock().unwrap();

        match guard.1 {
            RegistrationState::REGISTERED(_, e) => {
                if expiration == e {
                    return;
                }
            }
            _ => {}
        }
    }

    platform_log(LOG_TAG, "expiration is good");

    let registration = Arc::clone(registration);

    let flow_manager_ = Arc::clone(flow_manager);

    let core_ = Arc::clone(core);
    let rt_ = Arc::clone(rt);

    let state_callback_ = Arc::clone(state_callback);

    rt.spawn(async move {
        sleep_until(expiration).await;

        let registration_ = Arc::clone(&registration);

        let core = Arc::clone(&core_);
        let rt = Arc::clone(&rt_);

        let mut guard = registration.reg_state.lock().unwrap();

        match guard.1 {
            RegistrationState::REGISTERED(_, e) => {
                // consider this scenario:
                //  Client sends REGISTER to server, server responds with a expiration of 1 day
                //  But suddenly, a shorten-ed reg flow event is triggered due to some unknown cause
                //  The REGISTER response is received after the reg event
                //  We cannot determine the real expiration now
                if expiration == e {
                    let req_message = make_request(
                        &registration.home_domain,
                        &registration.impu,
                        registration.call_id,
                        &registration.transport_address,
                        registration.registration_id,
                        registration.sip_instance_id,
                        &registration.authentication_type,
                        &mut guard,
                        false,
                    );

                    core.get_transaction_manager().send_request(
                        // ctrl_itf,
                        req_message,
                        &registration.transport,
                        RegisterContext {
                            failure_count: 0,
                            registration: registration_,
                            flow_manager: flow_manager_,
                            core: core_,
                            rt: rt_,
                            state_callback: state_callback_,
                        },
                        &rt,
                    );
                }
            }
            _ => {}
        }
    });
}

fn create_next_authorization_header(
    auth: &mut DigestAnswerParams,
    is_aka_algorithm: bool,
) -> Option<Vec<u8>> {
    if let Some(credentials) = &mut auth.credentials {
        if let Some((ref mut client_nonce, ref mut nc)) = credentials.client_nonce {
            *client_nonce = rand::create_raw_alpha_numeric_string(8);
            *nc += 1;
        }
    }

    let mut base_algorithm = None;

    if let Some(challenge) = &auth.challenge {
        base_algorithm = Some(&challenge.algorithm);
    } else if let Some(algorithm) = &auth.algorithm {
        base_algorithm = Some(algorithm);
    }

    if let Some(base_algorithm) = base_algorithm {
        if is_aka_algorithm {
            match base_algorithm.as_aka_algorithm() {
                Ok(aka) => {
                    if let Ok(v) = auth.make_authorization_header(Some(aka.algorithm), false, true) {
                        return Some(v);
                    }
                }
                Err(()) => {
                    platform_log(LOG_TAG, "create_next_authorization_header error: cannot get base algorithm as aka algorithm")
                }
            }
        } else {
            if let Ok(v) = auth.make_authorization_header(Some(base_algorithm), false, true) {
                return Some(v);
            }
        }
    }

    None
}

fn make_request(
    home_domain: &String,
    impu: &String,
    call_id: Uuid,
    transport_address: &String,
    registration_id: u32,
    sip_instance_id: Uuid,
    authentication_type: &AuthenticationType,
    guard: &mut MutexGuard<(u32, RegistrationState, DigestAnswerParams)>,
    de_register: bool,
) -> SipMessage {
    let mut req_message =
        SipMessage::new_request(REGISTER, format!("sip:{}", home_domain).as_bytes());

    req_message.add_header(Header::new(
        b"Call-ID",
        String::from(
            call_id
                .as_hyphenated()
                .encode_lower(&mut Uuid::encode_buffer()),
        ),
    ));

    guard.0 += 1;

    req_message.add_header(Header::new(b"CSeq", format!("{} REGISTER", guard.0)));

    let contact;
    if let Some(idx) = impu.find('@') {
        contact = &(impu[0..idx]);
    } else {
        contact = impu;
    }

    req_message.add_header(Header::new(
        b"Contact",
        format!(
            "<{}@{}>;+sip.instance=\"<urn:uuid:{}>\";+g.3gpp.icsi-ref=\"urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.session,urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.session.group,urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.msg,urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.largemsg,urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.systemmsg,urn%3Aurn-7%3A3gpp-service.ims.icsi.oma.cpm.filetransfer\";+g.3gpp.iari-ref=\"urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.ftthumb,urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.fthttp,urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.ftsms,urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.chatbot.sa,urn%3Aurn-7%3A3gpp-application.ims.iari.rcse.im,urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.geopush,urn%3Aurn-7%3A3gpp-application.ims.iari.rcs.geosm\";+g.gsma.rcs.cpm.pager-large;+g.gsma.rcs.botversion=\"#=1,#=2\";reg-id={};expires={}",
            contact,
            transport_address,
            sip_instance_id
                .hyphenated()
                .encode_lower(&mut Uuid::encode_buffer()),
            registration_id,
            if de_register {
                0
            } else {
                3000
            },
        ),
    ));

    if guard.2.credentials.is_some() {
        req_message.add_header(Header::new(
            b"Allow",
            b"NOTIFY, OPTIONS, INVITE, UPDATE, CANCEL, BYE, ACK, MESSAGE",
        ));

        req_message.add_header(Header::new(b"Supported", b"path,gruu"));
    }

    if let Some(authorizaiton) = create_next_authorization_header(
        &mut guard.2,
        match authentication_type {
            AuthenticationType::Aka(_, _, _) => true,
            AuthenticationType::Digest(_, _, _, _) => false,
        },
    ) {
        platform_log(
            LOG_TAG,
            format!(
                "create_next_authorization_header returns: {}",
                String::from_utf8_lossy(&authorizaiton)
            ),
        );
        req_message.add_header(Header::new(b"Authorization", authorizaiton));
    } else {
        req_message.add_header(Header::new(b"Authorization", match authentication_type {
            AuthenticationType::Aka(algorithm, realm, username) => {
                format!("Digest username=\"{}\",algorithm={},nonce=\"\",response=\"\",uri=\"sip:{}\",realm=\"{}\"", username, algorithm, realm, realm)
            }
            AuthenticationType::Digest(algorithm, realm, username, _) => {
                format!("Digest username=\"{}\",algorithm={},nonce=\"\",response=\"\",uri=\"sip:{}\",realm=\"{}\"", username, algorithm, realm, realm)
            }
        }));
    }

    let tag = rand::create_raw_alpha_numeric_string(16);
    let tag = String::from_utf8_lossy(&tag);

    req_message.add_header(Header::new(b"From", format!("<{}>;tag={}", impu, tag)));

    req_message.add_header(Header::new(b"To", format!("<{}>", impu)));

    req_message.add_header(Header::new(b"Accept-Encoding", b"gzip"));

    req_message
}

pub fn deregister(registration: &Arc<Registration>, core: &Arc<SipCore>, rt: &Arc<Runtime>) {
    platform_log(LOG_TAG, "calling registration->deregister()");

    let mut guard = registration.reg_state.lock().unwrap();

    match &guard.1 {
        RegistrationState::IDLE => return,
        RegistrationState::REQUESTING => return,
        RegistrationState::REGISTERED(_, _) => guard.1 = RegistrationState::RELEASED,
        RegistrationState::RELEASED => return,
    }

    let req_message = make_request(
        &registration.home_domain,
        &registration.impu,
        registration.call_id,
        &registration.transport_address,
        registration.registration_id,
        registration.sip_instance_id,
        &registration.authentication_type,
        &mut guard,
        true,
    );

    core.get_transaction_manager().send_request(
        req_message,
        &registration.transport,
        DeregisterContext {
            registration: Arc::clone(registration),
            core: Arc::clone(core),
            rt: Arc::clone(rt),
        },
        rt,
    );
}

struct DeregisterContext {
    registration: Arc<Registration>,
    core: Arc<SipCore>,
    rt: Arc<Runtime>,
}

impl ClientTransactionCallbacks for DeregisterContext {
    fn on_provisional_response(&self, _message: SipMessage) {}

    fn on_final_response(&self, message: SipMessage) {
        if let SipMessage::Response(l, _headers, _) = &message {
            platform_log(
                LOG_TAG,
                format!("de-registration->on_final_response(): {}", l.status_code),
            );
        }

        let tm = self.core.get_transaction_manager();
        tm.unregister_sip_transport(&self.registration.transport, &self.rt);
    }

    fn on_transport_error(&self) {
        platform_log(LOG_TAG, "de-registration->on_transport_error()");
        let tm = self.core.get_transaction_manager();
        tm.unregister_sip_transport(&self.registration.transport, &self.rt);
    }
}
