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

pub mod chat_bot;
pub mod conference;
pub mod connection;
pub mod connectivity;
pub mod contact;
pub mod context;
pub mod ffi;
pub mod messaging;
pub mod provisioning;
pub mod rcs_engine;

use std::ffi::{CStr, CString};
use std::ptr::{self, null_mut, NonNull};
use std::sync::{Arc, Mutex};

// use backtrace::Backtrace;

use libc::c_char;
use messaging::cpm::MessagingSessionHandle;
use rust_rcs_core::security::gba::GbaContext;
use rust_rcs_core::third_gen_pp::ThreeDigit;
use tokio::runtime::Runtime;

use rust_rcs_core::ffi::log::platform_log;

use chat_bot::ffi::{
    RetrieveChatbotInfoResultCallback, RetrieveChatbotInfoResultCallbackContext,
    RetrieveChatbotInfoResultCallbackContextWrapper, RetrieveSpecificChatbotsResultCallback,
    RetrieveSpecificChatbotsResultCallbackContext,
    RetrieveSpecificChatbotsResultCallbackContextWrapper, SearchChatbotResultCallback,
    SearchChatbotResultCallbackContext, SearchChatbotResultCallbackContextWrapper,
};
use conference::ffi::{
    MultiConferenceCreateResultCallback, MultiConferenceCreateResultCallbackContext,
    MultiConferenceCreateResultCallbackContextWrapper, MultiConferenceEventListener,
    MultiConferenceEventListenerContext, MultiConferenceEventListenerContextWrapper,
    MultiConferenceJoinResultCallback, MultiConferenceJoinResultCallbackContext,
    MultiConferenceJoinResultCallbackContextWrapper, MultiConferenceV1InviteHandlerContext,
    MultiConferenceV1InviteHandlerContextWrapper, MultiConferenceV1InviteHandlerFunction,
};
use conference::MultiConferenceV1;
use context::Context;
use ffi::{
    MessageCallback, MessageCallbackContext, MessageCallbackContextWrapper, StateChangeCallback,
    StateChangeCallbackContext, StateChangeCallbackContextWrapper,
};
use messaging::ffi::{
    get_recipient_type, DownloadFileProgressCallback, DownloadFileProgressCallbackContext,
    DownloadFileResultCallback, DownloadFileResultCallbackContext,
    DownloadFileResultCallbackContextWrapper, DownloadileProgressCallbackContextWrapper,
    MessageResultCallback, MessageResultCallbackContext, MessageResultCallbackContextWrapper,
    SendImdnReportResultCallback, SendImdnReportResultCallbackContext,
    SendImdnReportResultCallbackContextWrapper, UploadFileProgressCallback,
    UploadFileProgressCallbackContext, UploadFileProgressCallbackContextWrapper,
    UploadFileResultCallback, UploadFileResultCallbackContext,
    UploadFileResultCallbackContextWrapper,
};
use provisioning::device_configuration::{
    start_auto_config, DeviceConfigurationStatus, RetryReason,
};

use rcs_engine::RcsEngine;

const LOG_TAG: &str = "librust_rcs_client";

pub struct RcsRuntime {
    pub rt: Arc<Runtime>,
    // context: Arc<Context>,
}

impl RcsRuntime {
    pub fn new() -> Result<RcsRuntime, ()> {
        std::panic::set_hook(Box::new(|info| {
            platform_log(LOG_TAG, format!("panic! {}", info));

            // let bt = Backtrace::new();

            // platform_log(LOG_TAG, format!("{:?}", bt));
        }));

        // let context = Context::new("fs_root_dir");
        if let Ok(rt) = Runtime::new() {
            // return Ok(RcsRuntime { rt, context: Arc::new(context) });
            return Ok(RcsRuntime { rt: Arc::new(rt) });
        }

        Err(())
    }
}

pub struct RcsClient {
    subscription_id: i32,
    mcc: u16,
    mnc: u16,
    imsi: String,
    imei: String,
    msisdn: Option<String>,

    context: Arc<Context>,

    // cb: Option<StateChangeCallback>,
    // cb_context: Arc<Mutex<StateChangeCallbackContextWrapper>>,
    engine: RcsEngine,
    // engine_itf: RcsEngineInterface,
    engine_gba_context: Option<Arc<GbaContext>>,
}

impl RcsClient {
    pub fn new(
        subscription_id: i32,
        mcc: u16,
        mnc: u16,
        imsi: &str,
        imei: &str,
        msisdn: Option<&str>,
        context: Arc<Context>,
        state_cb: Option<StateChangeCallback>,
        state_cb_context: *mut StateChangeCallbackContext,
        message_cb: Option<MessageCallback>,
        message_cb_context: *mut MessageCallbackContext,
        multi_conference_v1_invite_handler: Option<MultiConferenceV1InviteHandlerFunction>,
        multi_conference_v1_invite_handler_context: *mut MultiConferenceV1InviteHandlerContext,
        rt: Arc<Runtime>,
    ) -> RcsClient {
        let state_cb_context = Arc::new(Mutex::new(StateChangeCallbackContextWrapper(
            NonNull::new(state_cb_context).unwrap(),
        )));
        let message_cb_context = Arc::new(Mutex::new(MessageCallbackContextWrapper(
            NonNull::new(message_cb_context).unwrap(),
        )));
        let multi_conference_v1_invite_handler_context =
            Arc::new(Mutex::new(MultiConferenceV1InviteHandlerContextWrapper(
                NonNull::new(multi_conference_v1_invite_handler_context).unwrap(),
            )));
        let context_ = Arc::clone(&context);
        let engine = RcsEngine::new(
            subscription_id,
            rt,
            context_,
            move |state| {
                if let Some(state_cb) = state_cb {
                    let state_cb_context = state_cb_context.lock().unwrap();
                    let state_cb_context_ptr = state_cb_context.0.as_ptr();
                    match state {
                        rcs_engine::RcsEngineRegistrationState::NONE => {
                            state_cb(0, state_cb_context_ptr)
                        }
                        rcs_engine::RcsEngineRegistrationState::AUTHENTICATED(_) => {
                            state_cb(1, state_cb_context_ptr)
                        }
                        rcs_engine::RcsEngineRegistrationState::MAINTAINED(_) => {
                            state_cb(2, state_cb_context_ptr)
                        }
                    }
                }
            },
            move |service_type,
                  session_handle,
                  contact_uri,
                  content_type,
                  content_body,
                  imdn_message_id,
                  cpim_date,
                  cpim_from| {
                if let Some(messag_cb) = message_cb {
                    let session_handle = match session_handle {
                        Some(session_handle) => Some(Box::new(session_handle)),
                        None => None,
                    };
                    let contact_uri = CString::new(contact_uri).unwrap();
                    let content_type = CString::new(content_type).unwrap();
                    let content_body = CString::new(content_body).unwrap();
                    let imdn_message_id = CString::new(imdn_message_id).unwrap();
                    let cpim_date = CString::new(cpim_date).unwrap();
                    if let Some(cpim_from) = cpim_from {
                        let cpim_from = CString::new(cpim_from).unwrap();
                        let message_cb_context = message_cb_context.lock().unwrap();
                        let message_cb_context_ptr = message_cb_context.0.as_ptr();
                        messag_cb(
                            service_type,
                            session_handle,
                            contact_uri.as_ptr(),
                            content_type.as_ptr(),
                            content_body.as_ptr(),
                            imdn_message_id.as_ptr(),
                            cpim_date.as_ptr(),
                            cpim_from.as_ptr(),
                            message_cb_context_ptr,
                        );
                    } else {
                        let message_cb_context = message_cb_context.lock().unwrap();
                        let message_cb_context_ptr = message_cb_context.0.as_ptr();
                        messag_cb(
                            service_type,
                            session_handle,
                            contact_uri.as_ptr(),
                            content_type.as_ptr(),
                            content_body.as_ptr(),
                            imdn_message_id.as_ptr(),
                            cpim_date.as_ptr(),
                            ptr::null(),
                            message_cb_context_ptr,
                        );
                    };
                }
            },
            move |conference_v1, offer_sdp, response_receiver| {
                if let Some(multi_conference_v1_invite_handler) = multi_conference_v1_invite_handler
                {
                    let multi_conference_v1_invite_handler_context =
                        multi_conference_v1_invite_handler_context.lock().unwrap();
                    let multi_conference_v1_invite_handler_context_ptr =
                        multi_conference_v1_invite_handler_context.0.as_ptr();
                    let offer_sdp_ptr = offer_sdp.as_ptr();
                    let offer_sdp_len = offer_sdp.len();
                    multi_conference_v1_invite_handler(
                        Some(Box::new(conference_v1)),
                        offer_sdp_ptr,
                        offer_sdp_len,
                        Some(Box::new(response_receiver)),
                        multi_conference_v1_invite_handler_context_ptr,
                    );
                }
            },
        );
        RcsClient {
            subscription_id,
            mcc,
            mnc,
            imsi: imsi.to_string(),
            imei: imei.to_string(),
            msisdn: match msisdn {
                Some(msisdn) => Some(String::from(msisdn)),
                None => None,
            },
            context,
            // cb,
            // cb_context: Arc::new(Mutex::new(StateChangeCallbackContextWrapper(
            //     NonNull::new(cb_context).unwrap(),
            // ))),
            engine,
            // engine_itf,
            engine_gba_context: None,
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn new_rcs_runtime() -> Option<Box<RcsRuntime>> {
    if let Ok(rcs_runtime) = RcsRuntime::new() {
        return Some(Box::new(rcs_runtime));
    }
    None
}

#[no_mangle]
pub unsafe extern "C" fn new_rcs_client(
    rcs_runtime: *mut RcsRuntime,
    subscription_id: i32,
    mcc: u16,
    mnc: u16,
    imsi: *const c_char,
    imei: *const c_char,
    msisdn: *const c_char,
    fs_root_dir: *const c_char,
    state_cb: Option<StateChangeCallback>,
    state_cb_context: *mut StateChangeCallbackContext,
    message_cb: Option<MessageCallback>,
    message_cb_context: *mut MessageCallbackContext,
    conference_v1_invite_handler: Option<MultiConferenceV1InviteHandlerFunction>,
    conference_v1_invite_handler_context: *mut MultiConferenceV1InviteHandlerContext,
) -> Option<Box<RcsClient>> {
    let imsi = CStr::from_ptr(imsi).to_string_lossy().into_owned();
    let imei = CStr::from_ptr(imei).to_string_lossy().into_owned();
    let msisdn = if msisdn.is_null() {
        None
    } else {
        Some(CStr::from_ptr(msisdn).to_string_lossy())
    };
    let fs_root_dir = CStr::from_ptr(fs_root_dir).to_string_lossy().into_owned();
    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        platform_log(LOG_TAG, "creating context");
        let rt = Arc::clone(&rcs_runtime.rt);
        let context = Arc::new(Context::new(&fs_root_dir, rt));
        return Some(Box::new(RcsClient::new(
            subscription_id,
            mcc,
            mnc,
            &imsi,
            &imei,
            msisdn.as_deref(),
            context,
            state_cb,
            state_cb_context,
            message_cb,
            message_cb_context,
            conference_v1_invite_handler,
            conference_v1_invite_handler_context,
            Arc::clone(&rcs_runtime.rt),
        )));
    }
    None
}

type AutoConfigProgressCallback =
    extern "C" fn(status_code: i32, context: *mut AutoConfigCallbackContext);

type AutoConfigResultCallback = extern "C" fn(
    status_code: i32,
    ims_config: *const c_char,
    rcs_config: *const c_char,
    extra: *const c_char,
    // extra_host: *const c_char,
    // extra_cookie: *const c_char,
    context: *mut AutoConfigCallbackContext,
);

#[repr(C)]
pub struct AutoConfigCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn auto_config_callback_context_release(context: *mut AutoConfigCallbackContext);
}

struct AutoConfigCallbackContextWrapper(NonNull<AutoConfigCallbackContext>);

impl Drop for AutoConfigCallbackContextWrapper {
    fn drop(&mut self) {
        #[cfg(any(
            all(feature = "android", target_os = "android"),
            all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
        ))]
        let cb_context = self.0.as_ptr();
        #[cfg(any(
            all(feature = "android", target_os = "android"),
            all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
        ))]
        unsafe {
            auto_config_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for AutoConfigCallbackContextWrapper {}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_start_config(
    rcs_runtime: *mut RcsRuntime,
    client: *const RcsClient,
    p_cb: Option<AutoConfigProgressCallback>,
    r_cb: Option<AutoConfigResultCallback>,
    cb_context: *mut AutoConfigCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_start_config()");

    let cb_context = AutoConfigCallbackContextWrapper(NonNull::new(cb_context).unwrap());

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        if let Some(client) = client.as_ref() {
            let subscription_id = client.subscription_id;

            let mcc = client.mcc;
            let mnc = client.mnc;
            let imsi = String::from(&client.imsi);
            let imei = String::from(&client.imei);

            let context = Arc::clone(&client.context);

            let p_cb_context = Arc::new(Mutex::new(cb_context));
            let r_cb_context: Arc<Mutex<AutoConfigCallbackContextWrapper>> =
                Arc::clone(&p_cb_context);

            // let cb = client.cb;
            // let cb_context = client.cb_context.clone();
            rcs_runtime.rt.spawn(async move {
                match start_auto_config(
                    subscription_id,
                    mcc,
                    mnc,
                    &imei,
                    &imsi,
                    None,
                    context,
                    move |status| {
                        if let Some(cb) = p_cb {
                            let cb_context = p_cb_context.lock().unwrap();
                            let cb_context_ptr = cb_context.0.as_ptr();
                            cb(status, cb_context_ptr);
                        }
                    },
                )
                .await
                {
                    Ok((
                        msisdn,
                        rcs_application_characteristic,
                        ims_application_characteristic,
                    )) => {
                        platform_log(
                            LOG_TAG,
                            format!("on configuration success with provided msisdn {:?}", msisdn),
                        );

                        if let (
                            Ok(ims_application_characteristic_string),
                            Ok(rcs_application_characteristic_string),
                        ) = (
                            ims_application_characteristic.to_c_string(),
                            rcs_application_characteristic.to_c_string(),
                        ) {
                            platform_log(
                                LOG_TAG,
                                format!("with ims-app {:?}", ims_application_characteristic_string),
                            );

                            platform_log(
                                LOG_TAG,
                                format!("with rcs-app {:?}", rcs_application_characteristic_string),
                            );

                            // to-do: provide msisdn to callback
                            if let Some(cb) = r_cb {
                                let ims_config = ims_application_characteristic_string.as_ptr();
                                let rcs_config = rcs_application_characteristic_string.as_ptr();
                                let extra = ptr::null();
                                let cb_context: std::sync::MutexGuard<
                                    '_,
                                    AutoConfigCallbackContextWrapper,
                                > = r_cb_context.lock().unwrap();
                                let cb_context_ptr = cb_context.0.as_ptr();
                                cb(0, ims_config, rcs_config, extra, cb_context_ptr);
                            }
                        } else {
                            if let Some(cb) = r_cb {
                                let ims_config = ptr::null();
                                let rcs_config = ptr::null();
                                let extra = ptr::null();
                                let cb_context = r_cb_context.lock().unwrap();
                                let cb_context_ptr = cb_context.0.as_ptr();
                                cb(-1, ims_config, rcs_config, extra, cb_context_ptr);
                            }
                        }
                    }

                    Err(status) => {
                        platform_log(LOG_TAG, format!("on configuration error {:?}", status));

                        match status {
                            DeviceConfigurationStatus::InvalidArgument => {
                                if let Some(cb) = r_cb {
                                    let ims_config = ptr::null();
                                    let rcs_config = ptr::null();
                                    let extra = ptr::null();
                                    let cb_context = r_cb_context.lock().unwrap();
                                    let cb_context_ptr = cb_context.0.as_ptr();
                                    cb(-1, ims_config, rcs_config, extra, cb_context_ptr);
                                }
                            }

                            DeviceConfigurationStatus::Forbidden => {
                                if let Some(cb) = r_cb {
                                    let ims_config = ptr::null();
                                    let rcs_config = ptr::null();
                                    let extra = ptr::null();
                                    let cb_context = r_cb_context.lock().unwrap();
                                    let cb_context_ptr = cb_context.0.as_ptr();
                                    cb(-1, ims_config, rcs_config, extra, cb_context_ptr);
                                }
                            }

                            DeviceConfigurationStatus::Retry(reason, timeout) => {
                                if let Some(cb) = r_cb {
                                    let ims_config = ptr::null();
                                    let rcs_config = ptr::null();
                                    let extra = ptr::null();
                                    let cb_context = r_cb_context.lock().unwrap();
                                    let cb_context_ptr = cb_context.0.as_ptr();
                                    cb(-1, ims_config, rcs_config, extra, cb_context_ptr);
                                }
                            } // DeviceConfigurationStatus::Retry(reason, timeout) => match reason {
                              //     RetryReason::RequireOtp(port, session_host, session_cookie) => {
                              //         if let Some(cb) = cb {
                              //             let ims_config = ptr::null();
                              //             let rcs_config = ptr::null();
                              //             let session_host = session_host.as_str();
                              //             let session_cookie = session_cookie.as_str();
                              //             if let (Ok(extra_host), Ok(extra_cookie)) = (CString::new(session_host), CString::new(session_cookie)) {
                              //                 let extra_host = extra_host.as_ptr();
                              //                 let extra_cookie = extra_cookie.as_ptr();
                              //                 let cb_context = cb_context.lock().unwrap();
                              //                 let cb_context_ptr: *mut AutoConfigCallbackContext =
                              //                     cb_context.0.as_ptr();
                              //                 if port == 0 {
                              //                     cb(2, ims_config, rcs_config, extra_host, extra_cookie, cb_context_ptr);
                              //                 } else {
                              //                     cb(1, ims_config, rcs_config, extra_host, extra_cookie, cb_context_ptr);
                              //                 }
                              //             }
                              //         }
                              //     }
                              //     _ => {
                              //         if let Some(cb) = cb {
                              //             let ims_config = ptr::null();
                              //             let rcs_config = ptr::null();
                              //             let extra_host = ptr::null();
                              //             let extra_cookie = ptr::null();
                              //             let cb_context = cb_context.lock().unwrap();
                              //             let cb_context_ptr = cb_context.0.as_ptr();
                              //             cb(-1, ims_config, rcs_config, extra_host, extra_cookie, cb_context_ptr);
                              //         }
                              //     }
                              // },
                        }
                    }
                }
            });
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_input_otp(
    rcs_runtime: *mut RcsRuntime,
    client: *const RcsClient,
    otp: *const c_char,
) {
    platform_log(LOG_TAG, "calling rcs_client_input_otp()");

    if let Some(client) = client.as_ref() {
        let otp = CStr::from_ptr(otp).to_string_lossy().into_owned();

        client.context.broadcast_otp(&otp);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_setup(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    ims_config: *const c_char,
    rcs_config: *const c_char,
) {
    platform_log(LOG_TAG, "calling rcs_client_setup()");

    if let Some(client) = client.as_mut() {
        let ims_config = CStr::from_ptr(ims_config).to_string_lossy().into_owned();
        let rcs_config = CStr::from_ptr(rcs_config).to_string_lossy().into_owned();

        let gba_context = client.context.make_gba_context(
            &client.imsi,
            client.mcc,
            client.mnc,
            client.subscription_id,
        );
        let gba_context = Arc::new(gba_context);

        client.engine.configure(ims_config, rcs_config);
        client.engine_gba_context.replace(gba_context);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_connect(rcs_runtime: *mut RcsRuntime, client: *mut RcsClient) {
    platform_log(LOG_TAG, "calling rcs_client_connect()");

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        let rt = Arc::clone(&rcs_runtime.rt);
        if let Some(client) = client.as_mut() {
            client.engine.connect(rt);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_disconnect(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
) {
    platform_log(LOG_TAG, "calling rcs_client_disconnect()");

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        let rt = Arc::clone(&rcs_runtime.rt);
        if let Some(client) = client.as_mut() {
            client.engine.disconnect(rt);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_send_message(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    message_type: *const c_char,
    message_content: *const c_char,
    recipient: *const c_char,
    recipient_type: i8,
    cb: Option<MessageResultCallback>,
    cb_context: *mut MessageResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_send_message()");

    let cb_context = if cb_context.is_null() {
        None
    } else {
        Some(MessageResultCallbackContextWrapper(
            NonNull::new(cb_context).unwrap(),
        ))
    };

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        if let Some(client) = client.as_ref() {
            let message_type = CStr::from_ptr(message_type).to_string_lossy();
            let message_content = CStr::from_ptr(message_content).to_string_lossy();
            let recipient = CStr::from_ptr(recipient).to_string_lossy();
            let recipient_type = get_recipient_type(recipient_type);

            if let Some(recipient_type) = recipient_type {
                let cb_context_ = if let Some(cb_context) = cb_context {
                    Some(Arc::new(Mutex::new(cb_context)))
                } else {
                    None
                };

                client.engine.send_message(
                    &message_type,
                    &message_content,
                    &recipient,
                    recipient_type,
                    move |status_code, reason_phrase| {
                        if let Some(cb) = cb {
                            let reason_phrase = CString::new(reason_phrase).unwrap();
                            if let Some(cb_context_) = cb_context_ {
                                let cb_context = cb_context_.lock().unwrap();
                                cb(status_code, reason_phrase.as_ptr(), cb_context.0.as_ptr());
                            } else {
                                cb(status_code, reason_phrase.as_ptr(), ptr::null_mut());
                            }
                        }
                    },
                    &rcs_runtime.rt,
                );

                return;
            }
        }
    }

    if let Some(cb) = cb {
        let reason_phrase = CString::new("Forbidden").unwrap();
        cb(
            403,
            reason_phrase.as_ptr(),
            if let Some(cb_context) = cb_context {
                cb_context.0.as_ptr()
            } else {
                ptr::null_mut()
            },
        );
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_send_imdn_report(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    imdn_content: *const c_char,
    sender_uri: *const c_char,
    sender_service_type: i32,
    sender_session_handle: *mut MessagingSessionHandle,
    cb: Option<SendImdnReportResultCallback>,
    cb_context: *mut SendImdnReportResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_send_imdn_report()");

    let cb_context = if cb_context.is_null() {
        None
    } else {
        Some(SendImdnReportResultCallbackContextWrapper(
            NonNull::new(cb_context).unwrap(),
        ))
    };

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        let rt = Arc::clone(&rcs_runtime.rt);
        if let Some(client) = client.as_ref() {
            let cb_context_ = if let Some(cb_context) = cb_context {
                Some(Arc::new(Mutex::new(cb_context)))
            } else {
                None
            };

            let imdn_content = CStr::from_ptr(imdn_content).to_string_lossy();
            let sender_uri = CStr::from_ptr(sender_uri).to_string_lossy();

            client.engine.send_imdn_report(
                &imdn_content,
                &sender_uri,
                sender_service_type,
                sender_session_handle,
                rt,
                move |status_code, reason_phrase| {
                    if let Some(cb) = cb {
                        let reason_phrase = CString::new(reason_phrase).unwrap();
                        if let Some(cb_context_) = cb_context_ {
                            let cb_context = cb_context_.lock().unwrap();
                            cb(status_code, reason_phrase.as_ptr(), cb_context.0.as_ptr());
                        } else {
                            cb(status_code, reason_phrase.as_ptr(), ptr::null_mut());
                        }
                    }
                },
            );

            return;
        }
    }

    if let Some(cb) = cb {
        let reason_phrase = CString::new("Forbidden").unwrap();
        cb(
            403,
            reason_phrase.as_ptr(),
            if let Some(cb_context) = cb_context {
                cb_context.0.as_ptr()
            } else {
                ptr::null_mut()
            },
        );
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_upload_file(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    tid: *const c_char,
    file_path: *const c_char,
    file_name: *const c_char,
    file_mime: *const c_char,
    file_hash: *const c_char,
    thumbnail_path: *const c_char,
    thumbnail_name: *const c_char,
    thumbnail_mime: *const c_char,
    thumbnail_hash: *const c_char,
    progress_cb: Option<UploadFileProgressCallback>,
    progress_cb_context: *mut UploadFileProgressCallbackContext,
    result_cb: Option<UploadFileResultCallback>,
    result_cb_context: *mut UploadFileResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_upload_file()");

    let progress_cb_context = if progress_cb_context.is_null() {
        None
    } else {
        Some(UploadFileProgressCallbackContextWrapper(
            NonNull::new(progress_cb_context).unwrap(),
        ))
    };

    let result_cb_context = if result_cb_context.is_null() {
        None
    } else {
        Some(UploadFileResultCallbackContextWrapper(
            NonNull::new(result_cb_context).unwrap(),
        ))
    };

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        let rt = Arc::clone(&rcs_runtime.rt);
        if let Some(client) = client.as_ref() {
            let msisdn = client.msisdn.as_deref();

            if let Some(gba_context) = &client.engine_gba_context {
                let gba_context = Arc::clone(gba_context);
                let http_client = client.context.get_http_client();
                let security_context = client.context.get_security_context();

                let tid = CStr::from_ptr(tid).to_string_lossy();

                let file_path = CStr::from_ptr(file_path).to_string_lossy();
                let file_name = CStr::from_ptr(file_name).to_string_lossy();
                let file_mime = CStr::from_ptr(file_mime).to_string_lossy();
                let file_hash = if file_hash.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(file_hash).to_string_lossy())
                };

                let thumbnail_path = if thumbnail_path.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(thumbnail_path).to_string_lossy())
                };

                let thumbnail_name = if thumbnail_name.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(thumbnail_name).to_string_lossy())
                };

                let thumbnail_mime = if thumbnail_mime.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(thumbnail_mime).to_string_lossy())
                };

                let thumbnail_hash = if thumbnail_hash.is_null() {
                    None
                } else {
                    Some(CStr::from_ptr(thumbnail_hash).to_string_lossy())
                };

                let progress_cb_context_ = if let Some(progress_cb_context) = progress_cb_context {
                    Some(Arc::new(Mutex::new(progress_cb_context)))
                } else {
                    None
                };

                let result_cb_context_ = if let Some(result_cb_context) = result_cb_context {
                    Some(Arc::new(Mutex::new(result_cb_context)))
                } else {
                    None
                };

                client.engine.upload_file(
                    &tid,
                    &file_path,
                    &file_name,
                    &file_mime,
                    file_hash.as_deref(),
                    thumbnail_path.as_deref(),
                    thumbnail_name.as_deref(),
                    thumbnail_mime.as_deref(),
                    thumbnail_hash.as_deref(),
                    msisdn,
                    http_client,
                    gba_context,
                    security_context,
                    rt,
                    move |current, total| {
                        platform_log(
                            LOG_TAG,
                            format!("upload_file_progress_callback() {}/{}", current, total),
                        );
                        if let Some(progress_cb) = progress_cb {
                            if let Some(progress_cb_context_) = &progress_cb_context_ {
                                let progress_cb_context = progress_cb_context_.lock().unwrap();
                                progress_cb(current, total, progress_cb_context.0.as_ptr());
                            } else {
                                progress_cb(current, total, ptr::null_mut());
                            }
                        }
                    },
                    move |status_code, reason_phrase, result_xml| {
                        platform_log(
                            LOG_TAG,
                            format!(
                                "upload_file_result_callback() {} {} {:?}",
                                status_code, reason_phrase, result_xml
                            ),
                        );
                        if let Some(result_cb) = result_cb {
                            let reason_phrase = CString::new(reason_phrase).unwrap();
                            let result_xml = if let Some(result_xml) = result_xml {
                                let result_xml = CString::new(result_xml).unwrap();
                                Some(result_xml)
                            } else {
                                None
                            };
                            if let Some(result_cb_context_) = result_cb_context_ {
                                let result_cb_context = result_cb_context_.lock().unwrap();
                                result_cb(
                                    status_code,
                                    reason_phrase.as_ptr(),
                                    if let Some(result_xml) = &result_xml {
                                        result_xml.as_ptr()
                                    } else {
                                        ptr::null()
                                    },
                                    result_cb_context.0.as_ptr(),
                                );
                            } else {
                                result_cb(
                                    status_code,
                                    reason_phrase.as_ptr(),
                                    if let Some(result_xml) = &result_xml {
                                        result_xml.as_ptr()
                                    } else {
                                        ptr::null()
                                    },
                                    ptr::null_mut(),
                                );
                            }
                        }
                    },
                );

                return;
            }
        }
    }

    if let Some(result_cb) = result_cb {
        let reason_phrase = CString::new("Forbidden").unwrap();
        let xml = ptr::null();
        let result_cb_context_ptr = if let Some(result_cb_context) = result_cb_context {
            result_cb_context.0.as_ptr()
        } else {
            ptr::null_mut()
        };
        result_cb(403, reason_phrase.as_ptr(), xml, result_cb_context_ptr);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_download_file(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    file_uri: *const c_char,
    download_path: *const c_char,
    start: u32,
    total: i32,
    progress_cb: Option<DownloadFileProgressCallback>,
    progress_cb_context: *mut DownloadFileProgressCallbackContext,
    result_cb: Option<DownloadFileResultCallback>,
    result_cb_context: *mut DownloadFileResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_download_file()");

    let progress_cb_context = if progress_cb_context.is_null() {
        None
    } else {
        Some(DownloadileProgressCallbackContextWrapper(
            NonNull::new(progress_cb_context).unwrap(),
        ))
    };

    let result_cb_context = if result_cb_context.is_null() {
        None
    } else {
        Some(DownloadFileResultCallbackContextWrapper(
            NonNull::new(result_cb_context).unwrap(),
        ))
    };

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        let rt = Arc::clone(&rcs_runtime.rt);
        if let Some(client) = client.as_ref() {
            let msisdn = client.msisdn.as_deref();

            if let Some(gba_context) = &client.engine_gba_context {
                let gba_context = Arc::clone(gba_context);
                let http_client = client.context.get_http_client();
                let security_context = client.context.get_security_context();

                let file_uri = CStr::from_ptr(file_uri).to_string_lossy();
                let download_path = CStr::from_ptr(download_path).to_string_lossy();

                if let Ok(start) = usize::try_from(start) {
                    let total = if total > 0 {
                        if let Ok(total) = usize::try_from(total) {
                            Some(total)
                        } else {
                            None
                        }
                    } else {
                        None
                    };

                    let progress_cb_context_ =
                        if let Some(progress_cb_context) = progress_cb_context {
                            Some(Arc::new(Mutex::new(progress_cb_context)))
                        } else {
                            None
                        };

                    let result_cb_context_ = if let Some(result_cb_context) = result_cb_context {
                        Some(Arc::new(Mutex::new(result_cb_context)))
                    } else {
                        None
                    };

                    client.engine.download_file(
                        &file_uri,
                        &download_path,
                        start,
                        total,
                        msisdn,
                        http_client,
                        gba_context,
                        security_context,
                        rt,
                        move |current, total| {
                            if let Some(progress_cb) = progress_cb {
                                if let Some(progress_cb_context_) = &progress_cb_context_ {
                                    let progress_cb_context = progress_cb_context_.lock().unwrap();
                                    progress_cb(current, total, progress_cb_context.0.as_ptr());
                                } else {
                                    progress_cb(current, total, ptr::null_mut());
                                }
                            }
                        },
                        move |status_code, reason_phrase| {
                            if let Some(result_cb) = result_cb {
                                let reason_phrase = CString::new(reason_phrase).unwrap();
                                if let Some(result_cb_context_) = result_cb_context_ {
                                    let result_cb_context = result_cb_context_.lock().unwrap();
                                    result_cb(
                                        status_code,
                                        reason_phrase.as_ptr(),
                                        result_cb_context.0.as_ptr(),
                                    );
                                } else {
                                    result_cb(status_code, reason_phrase.as_ptr(), ptr::null_mut());
                                }
                            }
                        },
                    );

                    return;
                }
            }
        }
    }

    if let Some(cb) = result_cb {
        let reason_phrase = CString::new("Forbidden").unwrap();
        let result_cb_context_ptr = if let Some(result_cb_context) = result_cb_context {
            result_cb_context.0.as_ptr()
        } else {
            ptr::null_mut()
        };
        cb(403, reason_phrase.as_ptr(), result_cb_context_ptr);
    }
}

#[no_mangle]
pub unsafe extern "C" fn destroy_rcs_messaging_session(
    session_handle: *mut MessagingSessionHandle,
) {
    let _ = Box::from_raw(session_handle);
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_create_multi_conference_v1(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    recipients: *const c_char,
    offer_sdp: *const c_char,
    event_cb: Option<MultiConferenceEventListener>,
    event_cb_context: *mut MultiConferenceEventListenerContext,
    result_cb: Option<MultiConferenceCreateResultCallback>,
    result_cb_context: *mut MultiConferenceCreateResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_create_multi_conference_v1()");

    let event_cb_context =
        MultiConferenceEventListenerContextWrapper(NonNull::new(event_cb_context).unwrap());

    let result_cb_context =
        MultiConferenceCreateResultCallbackContextWrapper(NonNull::new(result_cb_context).unwrap());

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        if let Some(client) = client.as_ref() {
            let recipients = CStr::from_ptr(recipients).to_string_lossy();
            let offer_sdp = CStr::from_ptr(offer_sdp).to_string_lossy();

            let result_cb_context = Arc::new(Mutex::new(result_cb_context));

            client.engine.create_conference_v1(
                &recipients,
                &offer_sdp,
                event_cb,
                event_cb_context,
                &rcs_runtime.rt,
                move |result| {
                    if let Some(cb) = result_cb {
                        let cb_context = result_cb_context.lock().unwrap();
                        let cb_context_ptr = cb_context.0.as_ptr();
                        if let Some((conf, sdp)) = result {
                            let sdp_ptr = sdp.as_ptr();
                            let sdp_len = sdp.len();
                            cb(Some(Box::new(conf)), sdp_ptr, sdp_len, cb_context_ptr);
                        } else {
                            cb(None, ptr::null(), 0, cb_context_ptr);
                        }
                    }
                },
            );

            return;
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_join_multi_conference_v1(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    conference_id: *const c_char,
    offer_sdp: *const c_char,
    event_cb: Option<MultiConferenceEventListener>,
    event_cb_context: *mut MultiConferenceEventListenerContext,
    result_cb: Option<MultiConferenceJoinResultCallback>,
    result_cb_context: *mut MultiConferenceJoinResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_join_multi_conference_v1()");

    let _ = MultiConferenceEventListenerContextWrapper(NonNull::new(event_cb_context).unwrap());
    let _ =
        MultiConferenceJoinResultCallbackContextWrapper(NonNull::new(result_cb_context).unwrap());
}

// #[no_mangle]
// pub unsafe extern "C" fn rcs_multi_conference_register_event_listener(
//     rcs_runtime: *mut RcsRuntime,
//     conference: *mut MultiConferenceV1,
//     cb: Option<MultiConferenceEventCallback>,
//     cb_context: *mut MultiConferenceEventCallbackContext,
// ) {
//     let cb_context = MultiConferenceEventCallbackContextWrapper(NonNull::new(cb_context).unwrap());

//     if let Some(rcs_runtime) = rcs_runtime.as_ref() {
//         if let Some(conference) = conference.as_mut() {
//             if let Some(cb) = cb {
//                 conference.register_event_listener(cb, cb_context);
//             }
//         }
//     }
// }

#[no_mangle]
pub unsafe extern "C" fn rcs_multi_conference_v1_keep_alive(
    rcs_runtime: *mut RcsRuntime,
    conference: *mut MultiConferenceV1,
) {
    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        if let Some(conference) = conference.as_mut() {
            conference.keep_alive(&rcs_runtime.rt);
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_retrieve_specific_chatbots(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    local_etag: *const c_char,
    cb: Option<RetrieveSpecificChatbotsResultCallback>,
    cb_context: *mut RetrieveSpecificChatbotsResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_retrieve_specific_chatbots()");

    let cb_context =
        RetrieveSpecificChatbotsResultCallbackContextWrapper(NonNull::new(cb_context).unwrap());

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        let rt = Arc::clone(&rcs_runtime.rt);
        if let Some(client) = client.as_mut() {
            let local_etag = if local_etag.is_null() {
                None
            } else {
                Some(CStr::from_ptr(local_etag).to_string_lossy())
            };

            let msisdn = client.msisdn.as_deref();

            if let Some(gba_context) = &client.engine_gba_context {
                let gba_context = Arc::clone(gba_context);
                let http_client = client.context.get_http_client();
                let security_context = client.context.get_security_context();

                let cb_context_ = Arc::new(Mutex::new(cb_context));

                client.engine.retrieve_specific_chatbots(
                    local_etag.as_deref(),
                    msisdn,
                    http_client,
                    gba_context,
                    security_context,
                    rt,
                    move |status_code, reason_phrase, specific_chatbots, response_etag, expiry| {
                        if let Some(cb) = cb {
                            let reason_phrase = CString::new(reason_phrase).unwrap();

                            let specific_chatbots = match specific_chatbots {
                                Some(specific_chatbots) => {
                                    Some(CString::new(specific_chatbots).unwrap())
                                }
                                None => None,
                            };
                            let response_etag = match response_etag {
                                Some(response_etag) => Some(CString::new(response_etag).unwrap()),
                                None => None,
                            };

                            let cb_context = cb_context_.lock().unwrap();
                            let cb_context_ptr = cb_context.0.as_ptr();
                            cb(
                                status_code,
                                reason_phrase.as_ptr(),
                                if let Some(specific_chatbots) = &specific_chatbots {
                                    specific_chatbots.as_ptr()
                                } else {
                                    std::ptr::null()
                                },
                                if let Some(response_etag) = &response_etag {
                                    response_etag.as_ptr()
                                } else {
                                    std::ptr::null()
                                },
                                expiry,
                                cb_context_ptr,
                            );
                        }
                    },
                );

                return;
            }
        }
    }

    if let Some(cb) = cb {
        let reason_phrase = CString::new("Forbidden").unwrap();
        let cb_context_ptr = cb_context.0.as_ptr();
        cb(
            403,
            reason_phrase.as_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            cb_context_ptr,
        );
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_search_chatbot(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    query: *const c_char,
    start: u32,
    num: u32,
    cb: Option<SearchChatbotResultCallback>,
    cb_context: *mut SearchChatbotResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_search_chatbot()");

    let cb_context = SearchChatbotResultCallbackContextWrapper(NonNull::new(cb_context).unwrap());

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        let rt = Arc::clone(&rcs_runtime.rt);
        if let Some(client) = client.as_mut() {
            let query = CStr::from_ptr(query).to_string_lossy();
            let home_operator = format!("{}{}", client.mcc.string_repr(), client.mnc.string_repr());
            let msisdn = client.msisdn.as_deref();

            if let Some(gba_context) = &client.engine_gba_context {
                let gba_context = Arc::clone(gba_context);
                let http_client = client.context.get_http_client();
                let security_context = client.context.get_security_context();

                let cb_context_ = Arc::new(Mutex::new(cb_context));

                client.engine.search_chatbot(
                    &query,
                    start,
                    num,
                    &home_operator,
                    msisdn,
                    http_client,
                    gba_context,
                    security_context,
                    rt,
                    move |status_code, reason_phrase, json| {
                        if let Some(cb) = cb {
                            let reason_phrase = CString::new(reason_phrase).unwrap();
                            let json = match json {
                                Some(json) => Some(CString::new(json).unwrap()),
                                None => None,
                            };

                            let cb_context = cb_context_.lock().unwrap();
                            let cb_context_ptr = cb_context.0.as_ptr();
                            cb(
                                status_code,
                                reason_phrase.as_ptr(),
                                if let Some(json) = &json {
                                    json.as_ptr()
                                } else {
                                    std::ptr::null()
                                },
                                cb_context_ptr,
                            );
                        }
                    },
                );

                return;
            }
        }
    }

    if let Some(cb) = cb {
        let reason_phrase = CString::new("Forbidden").unwrap();
        let json = ptr::null();
        let cb_context_ptr = cb_context.0.as_ptr();
        cb(403, reason_phrase.as_ptr(), json, cb_context_ptr);
    }
}

#[no_mangle]
pub unsafe extern "C" fn rcs_client_retrieve_chatbot_info(
    rcs_runtime: *mut RcsRuntime,
    client: *mut RcsClient,
    chatbot_sip_uri: *const c_char,
    local_etag: *const c_char,
    cb: Option<RetrieveChatbotInfoResultCallback>,
    cb_context: *mut RetrieveChatbotInfoResultCallbackContext,
) {
    platform_log(LOG_TAG, "calling rcs_client_retrieve_chatbot_info()");

    let cb_context =
        RetrieveChatbotInfoResultCallbackContextWrapper(NonNull::new(cb_context).unwrap());

    if let Some(rcs_runtime) = rcs_runtime.as_ref() {
        let rt = Arc::clone(&rcs_runtime.rt);
        if let Some(client) = client.as_mut() {
            let chatbot_sip_uri = CStr::from_ptr(chatbot_sip_uri).to_string_lossy();
            let local_etag = if local_etag.is_null() {
                None
            } else {
                Some(CStr::from_ptr(local_etag).to_string_lossy())
            };
            let home_operator = format!("{}{}", client.mcc.string_repr(), client.mnc.string_repr());
            let home_language = String::from("zh"); // to-do: might want to pass it down
            let msisdn = client.msisdn.as_deref();

            if let Some(gba_context) = &client.engine_gba_context {
                let gba_context = Arc::clone(gba_context);
                let http_client = client.context.get_http_client();
                let security_context = client.context.get_security_context();

                let cb_context_ = Arc::new(Mutex::new(cb_context));

                client.engine.retrieve_chatbot_info(
                    &chatbot_sip_uri,
                    local_etag.as_deref(),
                    &home_operator,
                    &home_language,
                    msisdn,
                    http_client,
                    gba_context,
                    security_context,
                    rt,
                    move |status_code, reason_phrase, chatbot_info, response_etag, expiry| {
                        if let Some(cb) = cb {
                            let reason_phrase = CString::new(reason_phrase).unwrap();

                            let chatbot_info = match chatbot_info {
                                Some(chatbot_info) => Some(CString::new(chatbot_info).unwrap()),
                                None => None,
                            };
                            let response_etag = match response_etag {
                                Some(response_etag) => Some(CString::new(response_etag).unwrap()),
                                None => None,
                            };

                            let cb_context = cb_context_.lock().unwrap();
                            let cb_context_ptr = cb_context.0.as_ptr();
                            cb(
                                status_code,
                                reason_phrase.as_ptr(),
                                if let Some(chatbot_info) = &chatbot_info {
                                    chatbot_info.as_ptr()
                                } else {
                                    std::ptr::null()
                                },
                                if let Some(response_etag) = &response_etag {
                                    response_etag.as_ptr()
                                } else {
                                    std::ptr::null()
                                },
                                expiry,
                                cb_context_ptr,
                            );
                        }
                    },
                );

                return;
            }
        }
    }

    if let Some(cb) = cb {
        let reason_phrase = CString::new("Forbidden").unwrap();
        let cb_context_ptr = cb_context.0.as_ptr();
        cb(
            403,
            reason_phrase.as_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            cb_context_ptr,
        );
    }
}

#[no_mangle]
pub unsafe extern "C" fn destroy_rcs_client(client: *mut RcsClient) {
    let _ = Box::from_raw(client);
}
