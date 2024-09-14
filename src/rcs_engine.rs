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

extern crate libc;

use std::net::Ipv4Addr;
use std::net::Ipv6Addr;
#[cfg(not(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
)))]
use std::net::{SocketAddrV4, SocketAddrV6};
use std::sync::Arc;
use std::sync::Mutex;

use futures::StreamExt;

#[cfg(all(feature = "ohos", all(target_os = "linux", target_env = "ohos")))]
use libc::{AF_INET, AF_INET6};
use rust_rcs_core::ffi::log::platform_log;
use rust_rcs_core::http::HttpClient;
#[cfg(all(feature = "android", target_os = "android"))]
use rust_rcs_core::io::network::android_socket::AndroidTcpStream;
#[cfg(all(feature = "ohos", all(target_os = "linux", target_env = "ohos")))]
use rust_rcs_core::io::network::ohos_socket::OhosTcpStream;
use rust_rcs_core::io::network::stream::ClientSocket;
use rust_rcs_core::io::network::stream::ClientStream;

use rust_rcs_core::msrp::info::MsrpInfo;
use rust_rcs_core::msrp::info::MsrpInterfaceType;
use rust_rcs_core::msrp::info::MsrpSetupMethod;
use rust_rcs_core::security::gba::GbaContext;
use rust_rcs_core::security::SecurityContext;
use rust_rcs_core::sip::SipCore;
use rust_rcs_core::sip::SipTransactionManager;

use rust_rcs_core::sip::sip_subscription::SubscriptionManager;
use rust_rcs_core::sip::sip_transport::setup_sip_transport;
use rust_rcs_core::sip::SipTransport;
use rust_rcs_core::sip::TransactionHandler;
use rust_rcs_core::sip::ACK;
use rust_rcs_core::sip::BYE;
use rust_rcs_core::sip::CANCEL;
use rust_rcs_core::sip::INVITE;
use rust_rcs_core::sip::MESSAGE;
use rust_rcs_core::sip::NOTIFY;
use rust_rcs_core::sip::OPTIONS;
use rust_rcs_core::sip::UPDATE;

#[cfg(not(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
)))]
use tokio::net::TcpSocket;
use tokio::runtime::Runtime;

use tokio::sync::mpsc;
use tokio_stream::wrappers::ReceiverStream;

use url::Url;
use uuid::Uuid;

use crate::chat_bot;
use crate::chat_bot::chatbot_config::ChatbotConfig;
use crate::chat_bot::chatbot_sip_uri::AsChatbotSipUri;
use crate::chat_bot::RetrieveChatbotInfoSuccess;
use crate::chat_bot::RetrieveSpecificChatbotsSuccess;
use crate::conference::ffi::MultiConferenceEventListener;
use crate::conference::ffi::MultiConferenceEventListenerContextWrapper;
use crate::conference::MultiConferenceServiceV1;
use crate::conference::MultiConferenceServiceV1Wrapper;
use crate::conference::MultiConferenceV1;
use crate::conference::MultiConferenceV1InviteResponseReceiver;
use crate::connection::msrp_connection_config::MsrpConnectionConfig;
use crate::connection::p_cscf_connection_config::PCscfConnectionConfig;
use crate::connection::p_cscf_connection_config::ServiceType;
use crate::connectivity::authentication_type::AuthenticationType;
use crate::connectivity::registration::deregister;
use crate::connectivity::registration::start_register;
use crate::connectivity::registration::RegistrationEvent;
use crate::connectivity::{flow::FlowManager, registration::Registration};
use crate::context::Context;
use crate::messaging::config::MessagingConfigs;
use crate::messaging::cpm::session::CPMSessionService;
use crate::messaging::cpm::session::CPMSessionServiceWrapper;
use crate::messaging::cpm::standalone_messaging::StandaloneMessagingServiceWrapper;
use crate::messaging::cpm::standalone_messaging::{self, StandaloneMessagingService};
use crate::messaging::cpm::MessagingSessionHandle;
use crate::messaging::ffi::RecipientType;
use crate::messaging::ft_http::config::FileTransferOverHTTPConfigs;
use crate::messaging::ft_http::download_file;
use crate::messaging::ft_http::upload_file;
use crate::messaging::ft_http::FileInfo;
use crate::messaging::ft_http::FileTransferOverHTTPService;

use super::provisioning::characteristic::Characteristic;
use super::provisioning::ims_application::ImsApplication;
use super::provisioning::rcs_application::RcsApplication;

const LOG_TAG: &str = "librust_rcs_client";

pub enum RcsEngineConnectionState {
    IDLE,
    CONNECTING,
    CONNECTED(Arc<SipTransport>, String),
}

pub enum RcsEngineRegistrationState {
    NONE,
    AUTHENTICATED(String),
    MAINTAINED(String),
}

pub struct RcsEngine {
    state: Arc<Mutex<(RcsEngineConnectionState, RcsEngineRegistrationState)>>,

    state_callback: Arc<Box<dyn Fn(RcsEngineRegistrationState) + Send + Sync + 'static>>,

    tm: Arc<SipTransactionManager>,

    subscription_id: i32,

    impi: Option<String>,
    impu: Option<String>,

    home_domain: Option<String>,

    authentication_type: Option<AuthenticationType>,

    sip_instance_id: Uuid,

    p_cscf_connection_config: PCscfConnectionConfig,

    registration_id_counter: u32,

    flow_manager: Arc<FlowManager>,

    standalone_messaging_service: Arc<StandaloneMessagingService>,

    cpm_session_service: Arc<CPMSessionService>,

    ft_http_service: Arc<FileTransferOverHTTPService>,

    chatbot_config: ChatbotConfig,

    msrp_connection_config: Arc<Mutex<MsrpConnectionConfig>>,

    messaging_config: Arc<Mutex<MessagingConfigs>>,

    ft_http_configs: FileTransferOverHTTPConfigs,

    conference_service_v1: Arc<MultiConferenceServiceV1>,

    // msrp_channels: Arc<MsrpChannelManager>,
    core: Arc<SipCore>,
    context: Arc<Context>,
}

impl RcsEngine {
    pub fn new<SCF, MCF, MCIHF>(
        subscription_id: i32,
        rt: Arc<Runtime>,
        context: Arc<Context>,
        state_callback: SCF,
        message_callback: MCF,
        multi_conference_v1_invite_handler_function: MCIHF,
    ) -> RcsEngine
    where
        SCF: Fn(RcsEngineRegistrationState) + Send + Sync + 'static,
        MCF: Fn(i32, Option<MessagingSessionHandle>, &str, &str, &str, &str, &str, Option<&str>)
            + Send
            + Sync
            + 'static,
        MCIHF: Fn(MultiConferenceV1, String, MultiConferenceV1InviteResponseReceiver)
            + Send
            + Sync
            + 'static,
    {
        let state = Arc::new(Mutex::new((
            RcsEngineConnectionState::IDLE,
            RcsEngineRegistrationState::NONE,
        )));

        let state_callback: Arc<Box<dyn Fn(RcsEngineRegistrationState) + Send + Sync + 'static>> =
            Arc::new(Box::new(state_callback));
        let state_callback_ = Arc::clone(&state_callback);

        let sm = SubscriptionManager::new(500);
        // let (tm, tm_control_itf, tm_event_itf) = SipTransactionManager::new(rt);
        let (tm, tm_event_itf) = SipTransactionManager::new(&rt);

        #[cfg(not(all(feature = "android", target_os = "android")))]
        let tls_client_config = context.get_tls_client_config();

        let msrp_connection_config = MsrpConnectionConfig::new();
        let msrp_connection_config = Arc::new(Mutex::new(msrp_connection_config));
        let msrp_connection_config_ = Arc::clone(&msrp_connection_config);

        let messaging_config = Arc::new(Mutex::new(MessagingConfigs::new()));

        let messaging_config_1 = Arc::clone(&messaging_config);
        let messaging_config_2 = Arc::clone(&messaging_config);

        let msrp_socket_allocator_function_impl = move |msrp_info: Option<&MsrpInfo>| {
            if let Some((dns_config, mut tls)) = msrp_connection_config_
                .lock()
                .unwrap()
                .get_transport_config()
            {
                match msrp_info {
                    Some(msrp_info) => {
                        if tls && msrp_info.protocol.eq_ignore_ascii_case(b"TCP/MSRP") {
                            return Err((480, "Temporarily Unavailable"));
                        }

                        if msrp_info.protocol.eq_ignore_ascii_case(b"TCP/TLS/MSRP") {
                            tls = true; // it is okay to use tls transport under cellular IMHO
                        }

                        match msrp_info.setup_method {
                            MsrpSetupMethod::Passive => match msrp_info.interface_type {
                                MsrpInterfaceType::IPv4 => {
                                    #[cfg(all(feature = "android", target_os = "android"))]
                                    if let Ok(sock) = AndroidTcpStream::create() {
                                        // to-do: the socket
                                        if let Ok(_) =
                                            sock.bind(&Ipv4Addr::UNSPECIFIED.to_string(), 0)
                                        {
                                            if let Ok((ip, port)) = sock.get_local_addr() {
                                                return Ok((
                                                    ClientSocket(sock),
                                                    ip,
                                                    port,
                                                    tls,
                                                    true,
                                                    false,
                                                ));
                                            }
                                        }
                                    }

                                    #[cfg(all(
                                        feature = "ohos",
                                        all(target_os = "linux", target_env = "ohos")
                                    ))]
                                    if let Ok(sock) = OhosTcpStream::create() {
                                        if let Ok(_) = sock.bind(
                                            AF_INET,
                                            &Ipv4Addr::UNSPECIFIED.to_string(),
                                            0,
                                        ) {
                                            if let Ok((ip, port)) = sock.get_local_addr() {
                                                return Ok((
                                                    ClientSocket(sock),
                                                    ip,
                                                    port,
                                                    tls,
                                                    true,
                                                    false,
                                                ));
                                            }
                                        }
                                    }

                                    #[cfg(not(any(
                                        all(feature = "android", target_os = "android"),
                                        all(
                                            feature = "ohos",
                                            all(target_os = "linux", target_env = "ohos")
                                        )
                                    )))]
                                    if let Ok(sock) = TcpSocket::new_v4() {
                                        if let Ok(()) = sock.bind(std::net::SocketAddr::V4(
                                            SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, 0),
                                        )) {
                                            if let Ok(addr) = sock.local_addr() {
                                                let ip = match addr.ip() {
                                                    std::net::IpAddr::V4(v4) => format!("{}", v4),
                                                    std::net::IpAddr::V6(v6) => format!("{}", v6),
                                                };
                                                return Ok((
                                                    ClientSocket(sock),
                                                    ip,
                                                    addr.port(),
                                                    tls,
                                                    true,
                                                    false,
                                                ));
                                            }
                                        }
                                    }
                                }

                                MsrpInterfaceType::IPv6 => {
                                    #[cfg(all(feature = "android", target_os = "android"))]
                                    if let Ok(sock) = AndroidTcpStream::create() {
                                        if let Ok(_) =
                                            sock.bind(&Ipv6Addr::UNSPECIFIED.to_string(), 0)
                                        {
                                            if let Ok((ip, port)) = sock.get_local_addr() {
                                                return Ok((
                                                    ClientSocket(sock),
                                                    ip,
                                                    port,
                                                    tls,
                                                    true,
                                                    false,
                                                ));
                                            }
                                        }
                                    }

                                    #[cfg(all(
                                        feature = "ohos",
                                        all(target_os = "linux", target_env = "ohos")
                                    ))]
                                    if let Ok(sock) = OhosTcpStream::create() {
                                        if let Ok(_) = sock.bind(
                                            AF_INET6,
                                            &Ipv6Addr::UNSPECIFIED.to_string(),
                                            0,
                                        ) {
                                            if let Ok((ip, port)) = sock.get_local_addr() {
                                                return Ok((
                                                    ClientSocket(sock),
                                                    ip,
                                                    port,
                                                    tls,
                                                    true,
                                                    false,
                                                ));
                                            }
                                        }
                                    }

                                    #[cfg(not(any(
                                        all(feature = "android", target_os = "android"),
                                        all(
                                            feature = "ohos",
                                            all(target_os = "linux", target_env = "ohos")
                                        )
                                    )))]
                                    if let Ok(sock) = TcpSocket::new_v6() {
                                        if let Ok(()) = sock.bind(std::net::SocketAddr::V6(
                                            SocketAddrV6::new(Ipv6Addr::UNSPECIFIED, 0, 0, 0),
                                        )) {
                                            if let Ok(addr) = sock.local_addr() {
                                                let ip = match addr.ip() {
                                                    std::net::IpAddr::V4(v4) => format!("{}", v4),
                                                    std::net::IpAddr::V6(v6) => format!("{}", v6),
                                                };
                                                return Ok((
                                                    ClientSocket(sock),
                                                    ip,
                                                    addr.port(),
                                                    tls,
                                                    true,
                                                    false,
                                                ));
                                            }
                                        }
                                    }
                                }
                            },

                            MsrpSetupMethod::Active => {
                                // not supported yet
                            }
                        }
                    }

                    None => {
                        #[cfg(all(feature = "android", target_os = "android"))]
                        if let Ok(sock) = AndroidTcpStream::create() {
                            if let Ok(_) = sock.bind(&Ipv4Addr::UNSPECIFIED.to_string(), 0) {
                                if let Ok((ip, port)) = sock.get_local_addr() {
                                    return Ok((ClientSocket(sock), ip, port, tls, true, false));
                                }
                            }
                        }

                        #[cfg(all(
                            feature = "ohos",
                            all(target_os = "linux", target_env = "ohos")
                        ))]
                        if let Ok(sock) = OhosTcpStream::create() {
                            if let Ok(_) = sock.bind(AF_INET, &Ipv4Addr::UNSPECIFIED.to_string(), 0)
                            {
                                if let Ok((ip, port)) = sock.get_local_addr() {
                                    return Ok((ClientSocket(sock), ip, port, tls, true, false));
                                }
                            }
                        }

                        #[cfg(not(any(
                            all(feature = "android", target_os = "android"),
                            all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
                        )))]
                        if let Ok(sock) = TcpSocket::new_v4() {
                            if let Ok(()) = sock.bind(std::net::SocketAddr::V4(SocketAddrV4::new(
                                Ipv4Addr::UNSPECIFIED,
                                0,
                            ))) {
                                if let Ok(addr) = sock.local_addr() {
                                    let ip = match addr.ip() {
                                        std::net::IpAddr::V4(v4) => format!("{}", v4),
                                        std::net::IpAddr::V6(v6) => format!("{}", v6),
                                    };
                                    return Ok((
                                        ClientSocket(sock),
                                        ip,
                                        addr.port(),
                                        tls,
                                        true,
                                        false,
                                    ));
                                }
                            }
                        }

                        // to-do: could use ipv6 sometimes
                    }
                }
            }

            return Err((480, "Temporarily Unavailable"));
        };

        let msrp_socket_allocator_function_impl_1 = Arc::new(msrp_socket_allocator_function_impl);
        let msrp_socket_allocator_function_impl_2 =
            Arc::clone(&msrp_socket_allocator_function_impl_1);

        let msrp_socket_connect_function_impl =
            move |sock: ClientSocket, raddr: String, rport: u16, tls: bool| {
                #[cfg(not(all(feature = "android", target_os = "android")))]
                let tls_client_config_ = Arc::clone(&tls_client_config);
                return Box::pin(async move {
                    #[cfg(all(feature = "android", target_os = "android"))]
                    if let Ok(ip) = raddr.parse() {
                        if tls {
                            match sock.configure_tls(&raddr) {
                                Ok(sock) => {
                                    if let Ok(cs) = sock.connect(ip, rport).await {
                                        return Ok(cs);
                                    }
                                }
                                Err(_) => return Err((500, "")),
                            }
                        } else {
                            if let Ok(cs) = sock.connect(ip, rport).await {
                                return Ok(cs);
                            }
                        }
                    }

                    #[cfg(all(feature = "ohos", all(target_os = "linux", target_env = "ohos")))]
                    if let Ok(ip) = raddr.parse() {
                        if tls {
                            match sock.configure_tls(tls_client_config_, &raddr) {
                                Ok(cc) => {
                                    if let Ok(cs) = sock.connect(ip, rport, Some(cc)).await {
                                        return Ok(cs);
                                    }
                                }
                                Err(_) => return Err((500, "")),
                            }
                        } else {
                            if let Ok(cs) = sock.connect(ip, rport, None).await {
                                return Ok(cs);
                            }
                        }
                    }

                    #[cfg(not(any(
                        all(feature = "android", target_os = "android"),
                        all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
                    )))]
                    if let Ok(ip) = raddr.parse() {
                        if tls {
                            match sock.configure_tls(tls_client_config_, &raddr) {
                                Ok(cc) => {
                                    if let Ok(cs) = sock.connect(ip, rport, Some(cc)).await {
                                        return Ok(cs);
                                    }
                                }
                                Err(_) => return Err((500, "")),
                            }
                        } else {
                            if let Ok(cs) = sock.connect(ip, rport, None).await {
                                return Ok(cs);
                            }
                        }
                    }

                    Err((500, "Server Internal Error"))
                });
            };

        let message_callback_impl_1 = Arc::new(message_callback);
        let message_callback_impl_2 = Arc::clone(&message_callback_impl_1);

        let msrp_socket_connect_function_impl_1 = Arc::new(msrp_socket_connect_function_impl);
        let msrp_socket_connect_function_impl_2 = Arc::clone(&msrp_socket_connect_function_impl_1);

        let standalone_messaging_service = StandaloneMessagingService::new(
            move |contact_uri, cpim_info, content_type, message_body| {
                if let (Ok(contact_uri), Ok(content_type), Ok(message_body)) = (
                    std::str::from_utf8(contact_uri),
                    std::str::from_utf8(content_type),
                    std::str::from_utf8(message_body),
                ) {
                    if let (Ok(imdn_message_id), Ok(cpim_date)) = (
                        std::str::from_utf8(cpim_info.imdn_message_id),
                        std::str::from_utf8(cpim_info.date),
                    ) {
                        let cpim_from = if let Some(uri) = &cpim_info.from_uri {
                            let uri = uri.string_representation_without_query_and_fragment();
                            if let Ok(uri) = String::from_utf8(uri) {
                                Some(uri)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        message_callback_impl_1(
                            1,
                            None,
                            contact_uri,
                            content_type,
                            message_body,
                            imdn_message_id,
                            cpim_date,
                            cpim_from.as_deref(),
                        );
                    } else {
                        platform_log(LOG_TAG, "failed to decode cpim info as utf-8 string");
                    }
                } else {
                    platform_log(LOG_TAG, "failed to decode message data as utf-8 string");
                }
            },
            move |msrp_info| msrp_socket_allocator_function_impl_1(msrp_info),
            move |sock, raddr, rport, tls| {
                let raddr = String::from(raddr);
                msrp_socket_connect_function_impl_1(sock, raddr, rport, tls)
            },
        );
        let standalone_messaging_service = Arc::new(standalone_messaging_service);
        let cpm_session_service = CPMSessionService::new(
            move |session, contact_uri, cpim_info, content_type, message_body| {
                if let (Ok(contact_uri), Ok(content_type), Ok(message_body)) = (
                    std::str::from_utf8(contact_uri),
                    std::str::from_utf8(content_type),
                    std::str::from_utf8(message_body),
                ) {
                    if let (Ok(imdn_message_id), Ok(cpim_date)) = (
                        std::str::from_utf8(cpim_info.imdn_message_id),
                        std::str::from_utf8(cpim_info.date),
                    ) {
                        let cpim_from = if let Some(uri) = &cpim_info.from_uri {
                            let uri = uri.string_representation_without_query_and_fragment();
                            if let Ok(uri) = String::from_utf8(uri) {
                                Some(uri)
                            } else {
                                None
                            }
                        } else {
                            None
                        };
                        message_callback_impl_2(
                            0,
                            Some(MessagingSessionHandle {
                                // to-do: it might be deferred session
                                inner: Arc::clone(session),
                            }),
                            contact_uri,
                            content_type,
                            message_body,
                            imdn_message_id,
                            cpim_date,
                            cpim_from.as_deref(),
                        );
                    } else {
                        platform_log(LOG_TAG, "failed to decode cpim info as utf-8 string");
                    }
                } else {
                    platform_log(LOG_TAG, "failed to decode message data as utf-8 string");
                }
            },
            move |msrp_info| msrp_socket_allocator_function_impl_2(msrp_info),
            move |sock, raddr, rport, tls| {
                let raddr = String::from(raddr);
                msrp_socket_connect_function_impl_2(sock, raddr, rport, tls)
            },
            move |is_deferred_session, contact_uri, conversation_id, contribution_id, rx| {
                let messaging_config = &*messaging_config_1.lock().unwrap();
                if messaging_config.chat_auth == 1 {
                    if is_deferred_session {
                        return 200;
                    }
                    if messaging_config.im_session_auto_accept == 1 {
                        return 200;
                    }
                }
                180
            },
            move |contact_uri,
                  conversation_id,
                  contribution_id,
                  subject,
                  referred_by_name,
                  referred_by_uri,
                  rx| {
                let messaging_config = &*messaging_config_2.lock().unwrap();
                if messaging_config.group_chat_auth == 1 {
                    if messaging_config.im_session_auto_accept_group_chat == 1 {
                        return 200;
                    }
                }
                180
            },
            move |ev| {},
        );
        let cpm_session_service = Arc::new(cpm_session_service);
        let multi_conference_service_v1 =
            MultiConferenceServiceV1::new(multi_conference_v1_invite_handler_function);
        let multi_conference_service_v1 = Arc::new(multi_conference_service_v1);
        let sm = Arc::new(sm);
        let tm = Arc::new(tm);
        let allowed_methods: Vec<&'static [u8]> =
            [ACK, BYE, CANCEL, INVITE, MESSAGE, NOTIFY, OPTIONS, UPDATE].to_vec();

        let mut transaction_handlers: Vec<Box<dyn TransactionHandler + Send + Sync>> = Vec::new();
        transaction_handlers.push(Box::new(StandaloneMessagingServiceWrapper {
            service: Arc::clone(&standalone_messaging_service),
            tm: Arc::clone(&tm),
        }));
        transaction_handlers.push(Box::new(CPMSessionServiceWrapper {
            service: Arc::clone(&cpm_session_service),
            tm: Arc::clone(&tm),
        }));
        transaction_handlers.push(Box::new(MultiConferenceServiceV1Wrapper {
            service: Arc::clone(&multi_conference_service_v1),
            tm: Arc::clone(&tm),
        }));

        let core = Arc::new(SipCore::new(
            &sm,
            &tm,
            tm_event_itf,
            allowed_methods,
            transaction_handlers,
            &rt,
        ));

        let core_ = Arc::clone(&core);
        let rt_ = Arc::clone(&rt);

        RcsEngine {
            state: Arc::clone(&state),

            state_callback,

            tm: Arc::clone(&tm),

            subscription_id,

            impi: None,
            impu: None,

            home_domain: None,

            authentication_type: None,

            sip_instance_id: Uuid::new_v4(),

            p_cscf_connection_config: PCscfConnectionConfig::new(),

            registration_id_counter: 0,

            flow_manager: Arc::new(FlowManager::new(
                &sm,
                &tm,
                move |is_registered, transport, public_user_id, registration| {
                    platform_log(
                        LOG_TAG,
                        format!(
                            "flow_manager state_callback: is_registered={:?}, public_user_id={:?}",
                            is_registered, &public_user_id
                        ),
                    );

                    let mut guard = state.lock().unwrap();
                    match &guard.0 {
                        RcsEngineConnectionState::CONNECTED(transport_, _) => {
                            if Arc::ptr_eq(&transport, transport_) {
                                match (is_registered, public_user_id) {
                                    (true, Some(public_user_id)) => {
                                        let public_user_id_ = public_user_id.clone();
                                        guard.1 =
                                            RcsEngineRegistrationState::MAINTAINED(public_user_id_);
                                        state_callback_(RcsEngineRegistrationState::MAINTAINED(
                                            public_user_id,
                                        ));
                                    }
                                    _ => {
                                        guard.0 = RcsEngineConnectionState::IDLE;
                                        guard.1 = RcsEngineRegistrationState::NONE;
                                        state_callback_(RcsEngineRegistrationState::NONE);
                                    }
                                }

                                return;
                            }

                            platform_log(LOG_TAG, "already registered on another socket");
                        }
                        _ => {
                            platform_log(LOG_TAG, "already abandonned");
                        }
                    }

                    if is_registered {
                        deregister(&registration, &core_, &rt_);
                    }
                },
            )),

            standalone_messaging_service,

            cpm_session_service,

            ft_http_service: Arc::new(FileTransferOverHTTPService::new()),

            chatbot_config: ChatbotConfig::new(),

            msrp_connection_config,

            messaging_config,

            ft_http_configs: FileTransferOverHTTPConfigs::new(),

            conference_service_v1: multi_conference_service_v1,

            // msrp_channels,
            core,
            context,
            // gba_context: None,
        }
    }

    pub fn configure(&mut self, ims_config: String, rcs_config: String) {
        if let (Ok(ims_app), Ok(rcs_app)) = (
            ims_config.parse::<Characteristic>(),
            rcs_config.parse::<Characteristic>(),
        ) {
            let ims_app = ImsApplication::new(&ims_app);
            let rcs_app = RcsApplication::new(&rcs_app);

            self.impi.take();

            if let Some(pvui) = ims_app.get_private_user_identity() {
                self.impi.replace(String::from(pvui));
            }

            self.impu.take();

            let mut pbui_priority = 0;

            for pbui in ims_app.get_public_user_identity_list() {
                if pbui.starts_with("sip:") {
                    if pbui.starts_with("sip:+") {
                        if pbui_priority < 3 {
                            pbui_priority = 3;
                            self.impu = Some(String::from(pbui));
                        }
                    } else {
                        if pbui_priority < 2 {
                            pbui_priority = 2;
                            self.impu = Some(String::from(pbui));
                        }
                    }
                } else if pbui.starts_with("tel:") {
                    if pbui_priority < 1 {
                        pbui_priority = 1;
                        self.impu = Some(String::from(pbui));
                    }
                } else {
                    if self.impu.is_none() {
                        self.impu.replace(String::from(pbui));
                    }
                }
            }

            platform_log(
                LOG_TAG,
                format!(
                    "rcs engine configured with IMPI: {:?}, IMPU: {:?}",
                    &self.impi, &self.impu
                ),
            );

            self.home_domain = None;

            if let Some(home_domain) = ims_app.get_home_domain() {
                self.home_domain = Some(String::from(home_domain));
            }

            self.authentication_type = None;

            if let Some(gsma_ext) = ims_app.get_ims_gsma_extension() {
                let ext_info = gsma_ext.get_info();

                if let Some(auth_type) = ext_info.auth_type {
                    if auth_type.eq_ignore_ascii_case("AKA") {
                        if let (Some(realm), Some(username)) = (&self.home_domain, &self.impi) {
                            self.authentication_type = Some(AuthenticationType::Aka(
                                String::from("AKAv1-MD5"),
                                realm.clone(),
                                username.clone(),
                            ));
                        }
                    } else if auth_type.eq_ignore_ascii_case("Digest") {
                        if let (Some(realm), Some(username), Some(password)) =
                            (ext_info.realm, ext_info.user_name, ext_info.user_password)
                        {
                            self.authentication_type = Some(AuthenticationType::Digest(
                                String::from("SHA-256"),
                                String::from(realm),
                                String::from(username),
                                String::from(password),
                            ))
                        }
                    }
                }

                if let Some(uuid_value) = ext_info.uuid_value {
                    if let Ok(uuid) = Uuid::parse_str(uuid_value) {
                        self.sip_instance_id = uuid;
                    }
                }
            }

            self.p_cscf_connection_config.update_configuration(&ims_app);

            self.chatbot_config.update_configuration(&rcs_app);

            self.msrp_connection_config
                .lock()
                .unwrap()
                .update_configuration(&ims_app);

            self.messaging_config
                .lock()
                .unwrap()
                .update_configuration(&rcs_app);

            self.ft_http_configs.update_configuration(&rcs_app)
        }
    }

    pub fn connect(&mut self, rt: Arc<Runtime>) {
        platform_log(LOG_TAG, "calling engine->connect()");

        if let (Some(impi), Some(impu), Some(home_domain), Some(authentication_type)) = (
            &self.impi,
            &self.impu,
            &self.home_domain,
            &self.authentication_type,
        ) {
            if let Some((dns_config, service_type, address, known_host, known_ip, known_port)) =
                self.p_cscf_connection_config.get_next()
            {
                platform_log(
                    LOG_TAG,
                    format!(
                        "connect(): configured with: {}, known as {:?}, {:?}:{:?}",
                        &address, &known_host, &known_ip, &known_port
                    ),
                );

                let state = Arc::clone(&self.state);

                let mut guard = state.lock().unwrap();

                match &guard.0 {
                    RcsEngineConnectionState::IDLE => {
                        guard.0 = RcsEngineConnectionState::CONNECTING;
                    }

                    _ => {
                        return;
                    }
                }

                let impi = impi.clone();
                let impu = impu.clone();
                let home_domain = home_domain.clone();
                let authentication_type = authentication_type.clone();

                let state_ = Arc::clone(&state);
                let state_callback_ = Arc::clone(&self.state_callback);

                let tm = Arc::clone(&self.tm);
                // let tm_control_itf = engine_itf.tm_control_itf.clone();
                let tm_control_itf = tm.get_ctrl_itf();
                let context = Arc::clone(&self.context);
                let rt_ = Arc::clone(&rt);

                let subscription_id = self.subscription_id;
                let sip_instance_id = self.sip_instance_id;

                self.registration_id_counter += 1;
                let registration_id = self.registration_id_counter;

                let flow_manager = Arc::clone(&self.flow_manager);

                let cpm_session_service = Arc::clone(&self.cpm_session_service);

                let conference_service_v1 = Arc::clone(&self.conference_service_v1);

                let core = Arc::clone(&self.core);

                rt.spawn(async move {
                    let dns_client = context.get_dns_client();
                    let dns_config_ = dns_config.clone();

                    let service_name = service_type.get_string_repr();

                    #[cfg(not(all(feature = "android", target_os = "android")))]
                    let tls_client_config = context.get_tls_client_config();

                    if let Ok(mut stream) = if let Some(port) = known_port {
                        let (tx, rx) = mpsc::channel(1);
                        rt_.spawn(async move {
                            match tx.send((address.clone(), port)).await {
                                Ok(()) => {}
                                Err(_) => {}
                            }
                        });
                        Ok(ReceiverStream::from(rx))
                    } else {
                        dns_client
                            .resolve_service(dns_config_, address.clone(), service_name)
                            .await
                    } {
                        while let Some((target, port)) = stream.next().await {
                            platform_log(
                                LOG_TAG,
                                format!(
                                    "connect(): sip service resolved with {}:{}",
                                    &target, port
                                ),
                            );

                            let dns_config_ = dns_config.clone();

                            if let Ok(mut stream) = if let Some(addr) = known_ip {
                                let (tx, rx) = mpsc::channel(1);
                                rt_.spawn(async move {
                                    match tx.send(addr).await {
                                        Ok(()) => {}
                                        Err(_) => {}
                                    }
                                });
                                Ok(ReceiverStream::from(rx))
                            } else {
                                dns_client
                                    .resolve(
                                        dns_config_,
                                        if let Some(host) = &known_host {
                                            String::from(host)
                                        } else {
                                            target.clone()
                                        },
                                    )
                                    .await
                            } {
                                while let Some(addr) = stream.next().await {
                                    if let std::net::IpAddr::V6(_) = addr {
                                        platform_log(LOG_TAG, "IPv6 not supported for sip connection under most Carrier network now");
                                        continue;
                                    }
                                    if let Some(cs) = match service_type {
                                        ServiceType::SipD2T => {
                                            #[cfg(all(feature = "android", target_os = "android"))]
                                            let r = ClientStream::new_android(addr, port).await;

                                            #[cfg(all(feature = "ohos", all(target_os = "linux", target_env = "ohos")))]
                                            let r = ClientStream::new_ohos(addr, port).await;

                                            #[cfg(not(any(
                                                all(feature = "android", target_os = "android"),
                                                all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
                                            )))]
                                            let r = ClientStream::new_tokio(addr, port).await;

                                            match r {
                                                Ok(cs) => Some(cs),
                                                Err(e) => {
                                                    platform_log(LOG_TAG, format!("error creating client stream: {:?}", e));
                                                    None
                                                },
                                            }
                                        }
                                        ServiceType::SipsD2T => {
                                            #[cfg(all(feature = "android", target_os = "android"))]
                                            let r = ClientStream::new_android_ssl(
                                                addr,
                                                port,
                                                if let Some(host) = &known_host {
                                                    host
                                                } else {
                                                    &target
                                                },
                                            )
                                            .await;

                                            #[cfg(all(feature = "ohos", all(target_os = "linux", target_env = "ohos")))]
                                            let r = ClientStream::new_ohos_ssl(
                                                Arc::clone(&tls_client_config),
                                                addr,
                                                port,
                                                if let Some(host) = &known_host {
                                                    host
                                                } else {
                                                    &target
                                                },
                                            )
                                            .await;

                                            #[cfg(not(any(
                                                all(feature = "android", target_os = "android"),
                                                all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
                                            )))]
                                            let r = ClientStream::new_tokio_ssl(
                                                Arc::clone(&tls_client_config),
                                                addr,
                                                port,
                                                if let Some(host) = &known_host {
                                                    host
                                                } else {
                                                    &target
                                                },
                                            )
                                            .await;

                                            match r
                                            {
                                                Ok(cs) => match cs.do_handshake().await {
                                                    Ok((cs, _)) => {
                                                        platform_log(LOG_TAG, format!("ssl do_handshake success"));
                                                        Some(cs)
                                                    },
                                                    Err(e) => {
                                                        platform_log(
                                                            LOG_TAG,
                                                            format!("ssl do_handshake failed with error {:?}", e),
                                                        );
                                                        None
                                                    },
                                                }
,
                                                Err(e) => {
                                                    platform_log(LOG_TAG, format!("error creating ssl client stream: {:?}", e));
                                                    None
                                                },
                                            }
                                        }
                                        ServiceType::SipD2U => None,
                                    } {
                                        let transport_address = cs.get_local_transport_address();

                                        let t = SipTransport::new::<ClientStream>(
                                            transport_address.clone(),
                                            cs.get_sip_transport_type(),
                                        );

                                        let state_ec = Arc::clone(&state_);
                                        let flow_manager_ec = Arc::clone(&flow_manager);
                                        let tm_ec = Arc::clone(&tm);
                                        let rt_ec = Arc::clone(&rt_);
                                        let state_callback_ec = Arc::clone(&state_callback_);

                                        let transport = Arc::new(t);
                                        let transport_ec = Arc::clone(&transport);
                                        let transport_tx = setup_sip_transport(&transport, cs, tm_control_itf.clone(), Arc::clone(&rt_), move || {

                                            platform_log(
                                                LOG_TAG,
                                                "on sip transport exit",
                                            );

                                            let mut guard = state_ec.lock().unwrap();
                                            guard.0 = RcsEngineConnectionState::IDLE;
                                            guard.1 = RcsEngineRegistrationState::NONE;
                                            state_callback_ec(RcsEngineRegistrationState::NONE);
                                            let _ = flow_manager_ec.stop_observation(&transport_ec);
                                            tm_ec.unregister_sip_transport(&transport_ec, &rt_ec);
                                        });

                                        {
                                            let mut guard = state_.lock().unwrap();

                                            match guard.0 {
                                                RcsEngineConnectionState::CONNECTING => {
                                                    guard.0 = RcsEngineConnectionState::CONNECTED(
                                                        Arc::clone(&transport),
                                                        transport_address.clone(),
                                                    );

                                                    tm.register_sip_transport(Arc::clone(&transport), transport_tx);
                                                },

                                                _ => return,
                                            }
                                        }

                                        let state = Arc::clone(&state_);

                                        let flow_manager_ = Arc::clone(&flow_manager);

                                        let cpm_session_service_ = Arc::clone(&cpm_session_service);

                                        let conference_service_v1_ =
                                            Arc::clone(&conference_service_v1);

                                        let transport_ = Arc::clone(&transport);
                                        // let transport_address_ = transport_address.clone();

                                        let core_ = Arc::clone(&core);

                                        let rt = Arc::clone(&rt_);

                                        let registration = Registration::new(
                                            subscription_id,
                                            impi,
                                            impu,
                                            home_domain,
                                            authentication_type,
                                            Arc::clone(&transport),
                                            transport_address.clone(),
                                            registration_id,
                                            sip_instance_id,
                                        );

                                        let registration = Arc::new(registration);

                                        flow_manager.observe_registration(
                                            &transport,
                                            transport_address,
                                            &registration,
                                            &core_,
                                            &rt_,
                                        );

                                        start_register(&registration, &flow_manager, &core, &rt_, move |ev| {
                                            let transport = Arc::clone(&transport_);
                                            // let transport_address = transport_address_.clone();
                                            match ev {
                                                RegistrationEvent::Registered(
                                                    unbarred_impu,
                                                ) => {
                                                    platform_log(
                                                        LOG_TAG,
                                                        "on RegistrationEvent::Registered",
                                                    );

                                                    let sip_instance_id = format!(
                                                        "<urn:uuid:{}>",
                                                        sip_instance_id
                                                            .as_hyphenated()
                                                            .encode_lower(
                                                                &mut Uuid::encode_buffer()
                                                            )
                                                    );

                                                    core_.set_default_public_identity(
                                                        unbarred_impu.clone(),
                                                        sip_instance_id.clone(),
                                                        Arc::clone(&transport),
                                                    ); // fix-me: contact identities should not be visible for sip core, best we can do is to provide a map between transport and registered impu to each message consumer

                                                    cpm_session_service_
                                                        .set_registered_public_identity(
                                                            unbarred_impu.clone(),
                                                            sip_instance_id.clone(),
                                                            Arc::clone(&transport),
                                                        );

                                                    conference_service_v1_
                                                        .set_registered_public_identity(
                                                            unbarred_impu.clone(),
                                                            sip_instance_id.clone(),
                                                            Arc::clone(&transport),
                                                        );

                                                    let mut guard = state.lock().unwrap();
                                                    let default_public_identity =
                                                        unbarred_impu.clone();
                                                    guard.1 = RcsEngineRegistrationState::AUTHENTICATED(default_public_identity); // to-do: we need to append transport address anyway, why not here?
                                                    state_callback_(RcsEngineRegistrationState::AUTHENTICATED(unbarred_impu));
                                                },
                                                RegistrationEvent::Refreshed => {
                                                    platform_log(
                                                        LOG_TAG,
                                                        "on RegistrationEvent::Refreshed",
                                                    );
                                                },
                                                RegistrationEvent::Released => {
                                                    platform_log(
                                                        LOG_TAG,
                                                        "on RegistrationEvent::Released",
                                                    );

                                                    let mut guard = state.lock().unwrap();
                                                    guard.0 = RcsEngineConnectionState::IDLE;
                                                    guard.1 = RcsEngineRegistrationState::NONE;
                                                    state_callback_(RcsEngineRegistrationState::NONE);
                                                    let _ = flow_manager_.stop_observation(&transport);
                                                    tm.unregister_sip_transport(&transport, &rt);
                                                },
                                            }
                                        });

                                        return;
                                    }
                                }
                            }
                        }
                    }

                    let mut guard = state_.lock().unwrap();

                    guard.0 = RcsEngineConnectionState::IDLE;
                });
            }
        }
    }

    pub fn disconnect(&self, rt: Arc<Runtime>) {
        let mut guard = self.state.lock().unwrap();
        match &mut guard.0 {
            RcsEngineConnectionState::IDLE => {}
            RcsEngineConnectionState::CONNECTING => {}
            RcsEngineConnectionState::CONNECTED(transport, _) => {
                let transport = Arc::clone(transport);
                match &mut guard.1 {
                    RcsEngineRegistrationState::NONE => {}
                    RcsEngineRegistrationState::AUTHENTICATED(_)
                    | RcsEngineRegistrationState::MAINTAINED(_) => {
                        if let Some(registration) = self.flow_manager.stop_observation(&transport) {
                            deregister(&registration, &self.core, &rt);
                        }
                    }
                }
            }
        }

        guard.0 = RcsEngineConnectionState::IDLE;
        guard.1 = RcsEngineRegistrationState::NONE;
        (self.state_callback)(RcsEngineRegistrationState::NONE);
    }

    pub fn send_message<F>(
        &self,
        message_type: &str,
        message_content: &str,
        recipient: &str,
        recipient_type: RecipientType,
        message_result_callback: F,
        /* engine_itf: RcsEngineInterface, */ rt: &Arc<Runtime>,
    ) where
        F: FnOnce(u16, String) + Send + Sync + 'static,
    {
        let core = Arc::clone(&self.core);

        // let tm_control_itf = engine_itf.tm_control_itf;

        let guard = self.state.lock().unwrap();

        match &*guard {
            (
                RcsEngineConnectionState::CONNECTED(transport, _),
                RcsEngineRegistrationState::MAINTAINED(public_user_identity),
            ) => {
                let mut send_through_one_to_one_chat_service = false;
                let mut send_through_group_chat_service = false;
                let mut send_through_standalone_message_service = false;

                // bool contact_is_chatbot = (options & rcs_send_message_option_contact_is_chatbot) != 0;

                // bool contact_supports_one_to_one = (options & rcs_send_message_option_contact_supports_one_to_one) != 0;
                let recipient_supports_one_to_one = false;

                // bool contact_supports_standalone = (options & rcs_send_message_option_contact_supports_standalone) != 0;
                let recipient_supports_standalone = true;

                // bool contact_is_group = (options & rcs_send_message_option_send_to_group) != 0;
                // let recipient_is_group = false;

                let messaging_config = &*self.messaging_config.lock().unwrap();
                let mut recipient_uri = String::from(recipient);

                if let RecipientType::Chatbot = recipient_type {
                    if messaging_config.chatbot_msg_tech == 1
                        && messaging_config.chat_auth == 1
                        && recipient_supports_one_to_one
                    {
                        send_through_one_to_one_chat_service = true;
                    } else if messaging_config.chatbot_msg_tech == 2 {
                        if messaging_config.chat_auth == 1 && recipient_supports_one_to_one {
                            send_through_one_to_one_chat_service = true;
                        }

                        if messaging_config.standalone_msg_auth == 1
                            && recipient_supports_standalone
                        {
                            send_through_standalone_message_service = true;
                        }
                    } else if messaging_config.chatbot_msg_tech == 3
                        && messaging_config.standalone_msg_auth == 1
                        && recipient_supports_standalone
                    {
                        send_through_standalone_message_service = true;
                    }
                } else if let RecipientType::Group = recipient_type {
                    if messaging_config.group_chat_auth == 1 {
                        send_through_group_chat_service = true;
                    }
                } else if let RecipientType::ResourceList = recipient_type {
                    if messaging_config.standalone_msg_auth == 1 {
                        // to-do: check max_one_to_many_recipients for limit
                        if let Some(exploder_uri) = &messaging_config.exploder_uri {
                            recipient_uri = String::from(exploder_uri);
                        } else {
                            recipient_uri = String::from("exploder@conf-factory");
                        }
                        send_through_standalone_message_service = true;
                    }
                } else {
                    if messaging_config.chat_auth == 1 && recipient_supports_one_to_one {
                        send_through_one_to_one_chat_service = true;
                    }

                    if messaging_config.standalone_msg_auth == 1 && recipient_supports_standalone {
                        send_through_standalone_message_service = true;
                    }
                }

                if send_through_one_to_one_chat_service || send_through_group_chat_service {
                    let core = &self.core;
                    self.cpm_session_service.send_message(
                        message_type,
                        message_content,
                        recipient,
                        &recipient_type,
                        &recipient_uri,
                        message_result_callback,
                        core,
                        rt,
                    );

                    return;
                } else if send_through_standalone_message_service {
                    if messaging_config.standalone_msg_max_size >= message_content.len() {
                        if messaging_config.standalone_msg_switch_over_size < message_content.len()
                        {
                            let core = &self.core;
                            self.standalone_messaging_service.send_large_mode_message(
                                message_type,
                                message_content,
                                recipient,
                                &recipient_type,
                                &recipient_uri,
                                message_result_callback,
                                core,
                                rt,
                            );
                        } else {
                            standalone_messaging::send_message(
                                message_type,
                                message_content,
                                recipient,
                                &recipient_type,
                                &recipient_uri,
                                message_result_callback,
                                core,
                                transport,
                                public_user_identity,
                                &rt,
                            );
                        }

                        return;
                    }
                }
            }

            _ => {}
        }

        message_result_callback(403, String::from("Forbidden"))
    }

    pub fn send_imdn_report<F>(
        &self,
        imdn_content: &str,
        sender_uri: &str,
        sender_service_type: i32,
        sender_session_handle: *mut MessagingSessionHandle,
        rt: Arc<Runtime>,
        send_imdn_report_result_callback: F,
    ) where
        F: FnOnce(u16, String) + Send + Sync + 'static,
    {
        let guard = self.state.lock().unwrap();

        match &*guard {
            (
                RcsEngineConnectionState::CONNECTED(transport, _),
                RcsEngineRegistrationState::MAINTAINED(public_user_identity),
            ) => {
                if sender_service_type == 1 {
                    let core = Arc::clone(&self.core);

                    let recipient_type = RecipientType::Contact;

                    standalone_messaging::send_message(
                        "message/imdn",
                        imdn_content,
                        sender_uri,
                        &recipient_type, // to-do: support IMDN in group chat
                        sender_uri,
                        move |status_code, reason_phrase| {
                            send_imdn_report_result_callback(status_code, reason_phrase);
                        },
                        core,
                        transport,
                        public_user_identity,
                        &rt,
                    );

                    return;
                }
            }

            _ => {}
        }

        send_imdn_report_result_callback(403, String::from("Forbidden"));
    }

    pub fn upload_file<PF, RF>(
        &self,
        tid: &str,
        file_path: &str,
        file_name: &str,
        file_mime: &str,
        file_hash: Option<&str>,
        thumbnail_path: Option<&str>,
        thumbnail_name: Option<&str>,
        thumbnail_mime: Option<&str>,
        thumbnail_hash: Option<&str>,
        msisdn: Option<&str>,
        http_client: Arc<HttpClient>,
        gba_context: Arc<GbaContext>,
        security_context: Arc<SecurityContext>,
        rt: Arc<Runtime>,
        upload_file_progress_callback: PF,
        upload_file_result_callback: RF,
    ) where
        PF: Fn(u32, i32) + Send + Sync + 'static,
        RF: FnOnce(u16, String, Option<String>) + Send + Sync + 'static,
    {
        match Uuid::parse_str(tid) {
            Ok(tid) => {
                let ft_auth = self.ft_http_configs.ft_auth;

                if ft_auth == 1 {
                    if let Some(ft_http_cs_uri) = &self.ft_http_configs.ft_http_cs_uri {
                        let ft_http_service = Arc::clone(&self.ft_http_service);

                        let ft_http_cs_uri = String::from(ft_http_cs_uri);
                        let msisdn = match msisdn {
                            Some(msisdn) => Some(String::from(msisdn)),
                            None => None,
                        };

                        let file_path = String::from(file_path);
                        let file_name = String::from(file_name);
                        let file_mime = String::from(file_mime);
                        let file_hash = match file_hash {
                            Some(file_hash) => Some(String::from(file_hash)),
                            None => None,
                        };

                        let thumbnail_path = match thumbnail_path {
                            Some(thumbnail_path) => Some(String::from(thumbnail_path)),
                            None => None,
                        };

                        let thumbnail_name = match thumbnail_name {
                            Some(thumbnail_name) => Some(String::from(thumbnail_name)),
                            None => None,
                        };

                        let thumbnail_mime = match thumbnail_mime {
                            Some(thumbnail_mime) => Some(String::from(thumbnail_mime)),
                            None => None,
                        };

                        let thumbnail_hash = match thumbnail_hash {
                            Some(thumbnail_hash) => Some(String::from(thumbnail_hash)),
                            None => None,
                        };

                        rt.spawn(async move {
                            let file: FileInfo<'_> = FileInfo {
                                path: &file_path,
                                name: &file_name,
                                mime: &file_mime,
                                hash: file_hash.as_deref(),
                            };

                            let thumbnail = match (
                                thumbnail_path.as_deref(),
                                thumbnail_name.as_deref(),
                                thumbnail_mime.as_deref(),
                            ) {
                                (
                                    Some(thumbnail_path),
                                    Some(thumbnail_name),
                                    Some(thumbnail_mime),
                                ) => Some(FileInfo {
                                    path: thumbnail_path,
                                    name: thumbnail_name,
                                    mime: thumbnail_mime,
                                    hash: thumbnail_hash.as_deref(),
                                }),
                                _ => None,
                            };

                            let progress_callback: Box<dyn Fn(u32, i32) + Send + Sync> =
                                Box::new(upload_file_progress_callback);
                            let progress_callback = Arc::new(progress_callback);

                            match upload_file(
                                &ft_http_service,
                                &ft_http_cs_uri,
                                tid,
                                file,
                                thumbnail,
                                msisdn.as_deref(),
                                &http_client,
                                &gba_context,
                                &security_context,
                                &progress_callback,
                            )
                            .await
                            {
                                Ok(result_xml) => {
                                    upload_file_result_callback(
                                        200,
                                        String::from("Ok"),
                                        Some(result_xml),
                                    );
                                }

                                Err(e) => {
                                    let status_code = e.error_code();
                                    let reason_phrase = e.error_string();
                                    upload_file_result_callback(status_code, reason_phrase, None);
                                }
                            }
                        });

                        return;
                    }
                }

                upload_file_result_callback(403, String::from("Forbidden"), None);
            }

            Err(e) => upload_file_result_callback(400, format!("{}", e), None),
        }
    }

    pub fn download_file<PF, RF>(
        &self,
        file_uri: &str,
        download_path: &str,
        start: usize,
        total: Option<usize>,
        msisdn: Option<&str>,
        http_client: Arc<HttpClient>,
        gba_context: Arc<GbaContext>,
        security_context: Arc<SecurityContext>,
        rt: Arc<Runtime>,
        download_file_progress_callback: PF,
        download_file_result_callback: RF,
    ) where
        PF: Fn(u32, i32) + Send + Sync + 'static,
        RF: FnOnce(u16, String) + Send + Sync + 'static,
    {
        let ft_auth = self.ft_http_configs.ft_auth;

        if ft_auth == 1 {
            let file_uri = match &self.ft_http_configs.ft_http_dl_uri {
                Some(ft_http_dl_uri) => match Url::parse(ft_http_dl_uri) {
                    Ok(mut url) => {
                        let query = if let Some(query) = url.query() {
                            format!("{}&url={}", query, file_uri) // to-do: requires id, op, ci
                        } else {
                            format!("url={}", file_uri) // to-do: requires id, op, ci
                        };

                        url.set_query(Some(&query));

                        url.to_string()
                    }
                    Err(_) => String::from(file_uri),
                },
                None => String::from(file_uri),
            };

            let download_path = String::from(download_path);

            let msisdn = match msisdn {
                Some(msisdn) => Some(String::from(msisdn)),
                None => None,
            };

            rt.spawn(async move {
                match download_file(
                    &file_uri,
                    &download_path,
                    start,
                    total,
                    msisdn.as_deref(),
                    &http_client,
                    &gba_context,
                    &security_context,
                    None,
                    download_file_progress_callback,
                )
                .await
                {
                    Ok(()) => {
                        download_file_result_callback(200, String::from("Ok"));
                    }

                    Err(e) => {
                        let status_code = e.error_code();
                        let reason_phrase = e.error_string();
                        download_file_result_callback(status_code, reason_phrase);
                    }
                }
            });

            return;
        }

        download_file_result_callback(403, String::from("Forbidden"));
    }

    pub fn create_conference_v1<F>(
        &self,
        recipients: &str,
        offer_sdp: &str,
        event_cb: Option<MultiConferenceEventListener>,
        event_cb_context: MultiConferenceEventListenerContextWrapper,
        rt: &Arc<Runtime>,
        callback: F,
    ) where
        F: FnOnce(Option<(MultiConferenceV1, String)>) + Send + Sync + 'static,
    {
        let core = Arc::clone(&self.core);

        // to-do: check RcsEngineRegistrationState

        self.conference_service_v1.create_conference(
            recipients,
            offer_sdp,
            event_cb,
            event_cb_context,
            &core,
            rt,
            callback,
        )
    }

    pub fn retrieve_specific_chatbots<F>(
        &self,
        local_etag: Option<&str>,
        msisdn: Option<&str>,
        http_client: Arc<HttpClient>,
        gba_context: Arc<GbaContext>,
        security_context: Arc<SecurityContext>,
        rt: Arc<Runtime>,
        retrieve_specific_chatbots_result_callback: F,
    ) where
        F: FnOnce(u16, String, Option<String>, Option<String>, u32) + Send + Sync + 'static,
    {
        platform_log(LOG_TAG, "calling retrieve_specific_chatbots()");

        if let Some(specific_chatbots_lists_url) = &self.chatbot_config.specific_chatbots_lists {
            let specific_chatbots_lists_url = String::from(specific_chatbots_lists_url);

            let local_etag: Option<String> = match local_etag {
                Some(local_etag) => Some(String::from(local_etag)),
                None => None,
            };

            let msisdn = match msisdn {
                Some(msisdn) => Some(String::from(msisdn)),
                None => None,
            };

            rt.spawn(async move {
                match chat_bot::retrieve_specific_chatbots(
                    &specific_chatbots_lists_url,
                    local_etag.as_deref(),
                    msisdn.as_deref(),
                    &http_client,
                    &gba_context,
                    &security_context,
                    None,
                )
                .await
                {
                    Ok(result) => match result {
                        RetrieveSpecificChatbotsSuccess::Ok(
                            specific_chatbots,
                            response_etag,
                            expiry,
                        ) => {
                            retrieve_specific_chatbots_result_callback(
                                200,
                                String::from("Ok"),
                                Some(specific_chatbots),
                                response_etag,
                                expiry,
                            );
                        }
                        RetrieveSpecificChatbotsSuccess::NotModified(response_etag, expiry) => {
                            retrieve_specific_chatbots_result_callback(
                                304,
                                String::from("Not Modified"),
                                None,
                                response_etag,
                                expiry,
                            );
                        }
                    },
                    Err(e) => {
                        platform_log(
                            LOG_TAG,
                            format!("retrieve_specific_chatbots error: {:?}", &e),
                        );
                        let status_code = e.error_code();
                        let reason_phrase = e.error_string();
                        retrieve_specific_chatbots_result_callback(
                            status_code,
                            reason_phrase,
                            None,
                            None,
                            0,
                        );
                    }
                }
            });
        } else {
            retrieve_specific_chatbots_result_callback(
                404,
                String::from("Not Found"),
                None,
                None,
                0,
            );
        }
    }

    pub fn search_chatbot<F>(
        &self,
        query: &str,
        start: u32,
        num: u32,
        home_operator: &str,
        msisdn: Option<&str>,
        http_client: Arc<HttpClient>,
        gba_context: Arc<GbaContext>,
        security_context: Arc<SecurityContext>,
        rt: Arc<Runtime>,
        chatbot_search_result_callback: F,
    ) where
        F: FnOnce(u16, String, Option<String>) + Send + Sync + 'static,
    {
        platform_log(LOG_TAG, "calling search_chatbot()");

        if let Some(chatbot_directory) = &self.chatbot_config.chatbot_directory {
            let chatbot_directory = String::from(chatbot_directory);
            let query = String::from(query);
            let home_operator = String::from(home_operator);
            let msisdn = match msisdn {
                Some(msisdn) => Some(String::from(msisdn)),
                None => None,
            };

            rt.spawn(async move {
                match chat_bot::search_chatbot_directory(
                    &chatbot_directory,
                    &query,
                    start,
                    num,
                    &home_operator,
                    msisdn.as_deref(),
                    &http_client,
                    &gba_context,
                    &security_context,
                    None,
                )
                .await
                {
                    Ok(json) => {
                        chatbot_search_result_callback(200, String::from("Ok"), Some(json));
                    }
                    Err(e) => {
                        platform_log(LOG_TAG, format!("search_chatbot_directory error: {:?}", &e));
                        let status_code = e.error_code();
                        let reason_phrase = e.error_string();
                        chatbot_search_result_callback(status_code, reason_phrase, None);
                    }
                }
            });

            return;
        }

        chatbot_search_result_callback(403, String::from("Forbidden"), None);
    }

    pub fn retrieve_chatbot_info<F>(
        &self,
        chatbot_sip_uri: &str,
        local_etag: Option<&str>,
        home_operator: &str,
        home_language: &str,
        msisdn: Option<&str>,
        http_client: Arc<HttpClient>,
        gba_context: Arc<GbaContext>,
        security_context: Arc<SecurityContext>,
        rt: Arc<Runtime>,
        retrieve_chatbot_info_result_callback: F,
    ) where
        F: FnOnce(u16, String, Option<String>, Option<String>, u32) + Send + Sync + 'static,
    {
        platform_log(LOG_TAG, "calling retrieve_chatbot_info()");

        let host = match &self.chatbot_config.bot_info_fqdn {
            Some(bot_info_fqdn) => String::from(bot_info_fqdn),

            None => {
                if let Some(chatbot_sip_uri) = chatbot_sip_uri.as_chatbot_sip_uri() {
                    format!(
                        "{}.{}",
                        chatbot_sip_uri.bot_platform, chatbot_sip_uri.bot_platform_domain
                    )
                } else {
                    retrieve_chatbot_info_result_callback(
                        403,
                        String::from("Forbidden"),
                        None,
                        None,
                        0,
                    );
                    return;
                }
            }
        };

        let chatbot_sip_uri = String::from(chatbot_sip_uri);

        let local_etag: Option<String> = match local_etag {
            Some(local_etag) => Some(String::from(local_etag)),
            None => None,
        };

        let home_operator = String::from(home_operator);
        let home_language = String::from(home_language);

        let msisdn = match msisdn {
            Some(msisdn) => Some(String::from(msisdn)),
            None => None,
        };

        rt.spawn(async move {
            match chat_bot::retrieve_chatbot_info(
                &host,
                &chatbot_sip_uri,
                local_etag.as_deref(),
                &home_operator,
                &home_language,
                msisdn.as_deref(),
                &http_client,
                &gba_context,
                &security_context,
                None,
            )
            .await
            {
                Ok(result) => match result {
                    RetrieveChatbotInfoSuccess::Ok(chatbot_info, response_etag, expiry) => {
                        retrieve_chatbot_info_result_callback(
                            200,
                            String::from("Ok"),
                            Some(chatbot_info),
                            response_etag,
                            expiry,
                        );
                    }
                    RetrieveChatbotInfoSuccess::NotModified(response_etag, expiry) => {
                        retrieve_chatbot_info_result_callback(
                            304,
                            String::from("Not Modified"),
                            None,
                            response_etag,
                            expiry,
                        );
                    }
                },
                Err(e) => {
                    platform_log(LOG_TAG, format!("retrieve_chatbot_info error: {:?}", &e));
                    let status_code = e.error_code();
                    let reason_phrase = e.error_string();
                    retrieve_chatbot_info_result_callback(
                        status_code,
                        reason_phrase,
                        None,
                        None,
                        0,
                    );
                }
            }
        });
    }
}
