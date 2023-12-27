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

use std::ptr::NonNull;

use libc::c_char;

pub type RetrieveSpecificChatbotsResultCallback = extern "C" fn(
    status_code: u16,
    reason_phrase: *const c_char,
    specific_chatbots: *const c_char,
    response_etag: *const c_char,
    expiry: u32,
    context: *mut RetrieveSpecificChatbotsResultCallbackContext,
);

#[repr(C)]
pub struct RetrieveSpecificChatbotsResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn retrieve_specific_chatbots_result_callback_context_release(
        context: *mut RetrieveSpecificChatbotsResultCallbackContext,
    );
}

pub struct RetrieveSpecificChatbotsResultCallbackContextWrapper(
    pub NonNull<RetrieveSpecificChatbotsResultCallbackContext>,
);

impl Drop for RetrieveSpecificChatbotsResultCallbackContextWrapper {
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
            retrieve_specific_chatbots_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for RetrieveSpecificChatbotsResultCallbackContextWrapper {}

pub type SearchChatbotResultCallback = extern "C" fn(
    status_code: u16,
    reason_phrase: *const c_char,
    chatbot_search_result_list_json: *const c_char,
    context: *mut SearchChatbotResultCallbackContext,
);

#[repr(C)]
pub struct SearchChatbotResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn search_chatbot_result_callback_context_release(
        context: *mut SearchChatbotResultCallbackContext,
    );
}

pub struct SearchChatbotResultCallbackContextWrapper(
    pub NonNull<SearchChatbotResultCallbackContext>,
);

impl Drop for SearchChatbotResultCallbackContextWrapper {
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
            search_chatbot_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for SearchChatbotResultCallbackContextWrapper {}

pub type RetrieveChatbotInfoResultCallback = extern "C" fn(
    status_code: u16,
    reason_phrase: *const c_char,
    chatbot_info: *const c_char,
    response_etag: *const c_char,
    expiry: u32,
    context: *mut RetrieveChatbotInfoResultCallbackContext,
);

#[repr(C)]
pub struct RetrieveChatbotInfoResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn retrieve_chatbot_info_result_callback_context_release(
        context: *mut RetrieveChatbotInfoResultCallbackContext,
    );
}

pub struct RetrieveChatbotInfoResultCallbackContextWrapper(
    pub NonNull<RetrieveChatbotInfoResultCallbackContext>,
);

impl Drop for RetrieveChatbotInfoResultCallbackContextWrapper {
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
            retrieve_chatbot_info_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for RetrieveChatbotInfoResultCallbackContextWrapper {}
