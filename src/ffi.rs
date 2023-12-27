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

use std::{ffi::c_char, ptr::NonNull};

use crate::messaging::cpm::MessagingSessionHandle;

pub type StateChangeCallback = extern "C" fn(state: i32, context: *mut StateChangeCallbackContext);

#[repr(C)]
pub struct StateChangeCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn state_change_callback_context_release(context: *mut StateChangeCallbackContext);
}

pub struct StateChangeCallbackContextWrapper(pub NonNull<StateChangeCallbackContext>);

impl Drop for StateChangeCallbackContextWrapper {
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
            state_change_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for StateChangeCallbackContextWrapper {}

pub type MessageCallback = extern "C" fn(
    i32,
    Option<Box<MessagingSessionHandle>>,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
    *const c_char,
    context: *mut MessageCallbackContext,
);

#[repr(C)]
pub struct MessageCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn message_callback_context_release(context: *mut MessageCallbackContext);
}

pub struct MessageCallbackContextWrapper(pub NonNull<MessageCallbackContext>);

impl Drop for MessageCallbackContextWrapper {
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
            message_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for MessageCallbackContextWrapper {}
