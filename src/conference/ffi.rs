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

use std::ptr::{self, NonNull};

use libc::c_char;

use super::{
    subscription::MultiConferenceEvent, MultiConferenceV1, MultiConferenceV1InviteResponseReceiver,
};

#[repr(C)]
pub struct MultiConferenceV1InviteResponse {
    pub status_code: u16,
    pub answer_sdp: *mut c_char,
    pub event_listener: Option<MultiConferenceEventListener>,
    pub event_listener_context: *mut MultiConferenceEventListenerContext,
}

pub type MultiConferenceV1InviteHandlerFunction = extern "C" fn(
    conf: Option<Box<MultiConferenceV1>>,
    offer_sdp: *const u8,
    offer_sdp_len: usize,
    response_receiver: Option<Box<MultiConferenceV1InviteResponseReceiver>>,
    context: *mut MultiConferenceV1InviteHandlerContext,
);

#[repr(C)]
pub struct MultiConferenceV1InviteHandlerContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn multi_conference_v1_invite_handler_context_release(
        handler_context: *mut MultiConferenceV1InviteHandlerContext,
    );
}

pub struct MultiConferenceV1InviteHandlerContextWrapper(
    pub NonNull<MultiConferenceV1InviteHandlerContext>,
);

impl Drop for MultiConferenceV1InviteHandlerContextWrapper {
    fn drop(&mut self) {
        #[cfg(any(
            all(feature = "android", target_os = "android"),
            all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
        ))]
        let handler_context = self.0.as_ptr();
        #[cfg(any(
            all(feature = "android", target_os = "android"),
            all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
        ))]
        unsafe {
            multi_conference_v1_invite_handler_context_release(handler_context);
        }
    }
}

unsafe impl Send for MultiConferenceV1InviteHandlerContextWrapper {}

#[no_mangle]
pub unsafe extern "C" fn multi_conference_v1_invite_response_receiver_send_response(
    receiver: *mut MultiConferenceV1InviteResponseReceiver,
    response: *mut MultiConferenceV1InviteResponse,
) {
    if let Some(receiver) = receiver.as_mut() {
        if let Some(response) = response.as_ref() {
            receiver.accept_result(&response);
        } else {
            receiver.drop_result();
        }
    }
}

#[no_mangle]
pub unsafe extern "C" fn free_multi_conference_v1_invite_response_receiver(
    receiver: *mut MultiConferenceV1InviteResponseReceiver,
) {
    let _ = Box::from_raw(receiver);
}

pub type MultiConferenceEventListener = extern "C" fn(
    event_type: u16,
    event: Option<Box<MultiConferenceEvent>>,
    context: *mut MultiConferenceEventListenerContext,
);

#[repr(C)]
pub struct MultiConferenceEventListenerContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn multi_conference_v1_event_listener_context_release(
        context: *mut MultiConferenceEventListenerContext,
    );
}

pub struct MultiConferenceEventListenerContextWrapper(
    pub NonNull<MultiConferenceEventListenerContext>,
);

impl Drop for MultiConferenceEventListenerContextWrapper {
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
            multi_conference_v1_event_listener_context_release(cb_context);
        }
    }
}

unsafe impl Send for MultiConferenceEventListenerContextWrapper {}

#[no_mangle]
pub unsafe extern "C" fn rcs_multi_conference_v1_event_get_user_joined(
    event: *mut MultiConferenceEvent,
) -> *mut c_char {
    if let Some(event) = event.as_ref() {
        if let Ok(user) = event.get_user_joined() {
            return user.into_raw();
        }
    }

    ptr::null::<c_char>() as *mut c_char
}

#[no_mangle]
pub unsafe extern "C" fn rcs_multi_conference_v1_event_get_user_left(
    event: *mut MultiConferenceEvent,
) -> *mut c_char {
    if let Some(event) = event.as_ref() {
        if let Ok(user) = event.get_user_left() {
            return user.into_raw();
        }
    }

    ptr::null::<c_char>() as *mut c_char
}

#[no_mangle]
pub unsafe extern "C" fn free_rcs_multi_conference_v1_event(event: *mut MultiConferenceEvent) {
    let _ = Box::from_raw(event);
}

pub type MultiConferenceCreateResultCallback = extern "C" fn(
    conference: Option<Box<MultiConferenceV1>>,
    sdp: *const u8,
    sdp_len: usize,
    context: *mut MultiConferenceCreateResultCallbackContext,
);

#[repr(C)]
pub struct MultiConferenceCreateResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn multi_conference_v1_create_result_callback_context_release(
        context: *mut MultiConferenceCreateResultCallbackContext,
    );
}

pub struct MultiConferenceCreateResultCallbackContextWrapper(
    pub NonNull<MultiConferenceCreateResultCallbackContext>,
);

impl Drop for MultiConferenceCreateResultCallbackContextWrapper {
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
            multi_conference_v1_create_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for MultiConferenceCreateResultCallbackContextWrapper {}

pub type MultiConferenceJoinResultCallback = extern "C" fn(
    conference: Option<Box<MultiConferenceV1>>,
    sdp: *const u8,
    sdp_len: usize,
    context: *mut MultiConferenceJoinResultCallbackContext,
);

#[repr(C)]
pub struct MultiConferenceJoinResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn multi_conference_v1_join_result_callback_context_release(
        context: *mut MultiConferenceJoinResultCallbackContext,
    );
}

pub struct MultiConferenceJoinResultCallbackContextWrapper(
    pub NonNull<MultiConferenceJoinResultCallbackContext>,
);

impl Drop for MultiConferenceJoinResultCallbackContextWrapper {
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
            multi_conference_v1_join_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for MultiConferenceJoinResultCallbackContextWrapper {}

#[no_mangle]
pub unsafe extern "C" fn destroy_rcs_multi_conference(conference_v1: *mut MultiConferenceV1) {
    let _ = Box::from_raw(conference_v1);
}
