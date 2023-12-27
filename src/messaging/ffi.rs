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

pub enum RecipientType {
    Contact,
    Chatbot,
    Group,
    ResourceList,
}

impl From<&RecipientType> for RecipientType {
    fn from(recipient_type: &RecipientType) -> RecipientType {
        match recipient_type {
            RecipientType::Contact => RecipientType::Contact,
            RecipientType::Chatbot => RecipientType::Chatbot,
            RecipientType::Group => RecipientType::Group,
            RecipientType::ResourceList => RecipientType::ResourceList,
        }
    }
}

pub fn get_recipient_type(recipient_type: i8) -> Option<RecipientType> {
    if recipient_type == 0 {
        Some(RecipientType::Contact)
    } else if recipient_type == 1 {
        Some(RecipientType::Chatbot)
    } else if recipient_type == 2 {
        Some(RecipientType::Group)
    } else if recipient_type == 3 {
        Some(RecipientType::ResourceList)
    } else {
        None
    }
}

pub type MessageResultCallback = extern "C" fn(
    status_code: u16,
    reason_phrase: *const c_char,
    context: *mut MessageResultCallbackContext,
);

#[repr(C)]
pub struct MessageResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn message_result_callback_context_release(context: *mut MessageResultCallbackContext);
}

pub struct MessageResultCallbackContextWrapper(pub NonNull<MessageResultCallbackContext>);

impl Drop for MessageResultCallbackContextWrapper {
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
            message_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for MessageResultCallbackContextWrapper {}

pub type SendImdnReportResultCallback = extern "C" fn(
    status_code: u16,
    reason_phrase: *const c_char,
    context: *mut SendImdnReportResultCallbackContext,
);

#[repr(C)]
pub struct SendImdnReportResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn send_imdn_report_result_callback_context_release(
        context: *mut SendImdnReportResultCallbackContext,
    );
}

pub struct SendImdnReportResultCallbackContextWrapper(
    pub NonNull<SendImdnReportResultCallbackContext>,
);

impl Drop for SendImdnReportResultCallbackContextWrapper {
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
            send_imdn_report_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for SendImdnReportResultCallbackContextWrapper {}

pub type UploadFileProgressCallback =
    extern "C" fn(current: u32, total: i32, context: *mut UploadFileProgressCallbackContext);

#[repr(C)]
pub struct UploadFileProgressCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn upload_file_progress_callback_context_release(
        context: *mut UploadFileProgressCallbackContext,
    );
}

pub struct UploadFileProgressCallbackContextWrapper(pub NonNull<UploadFileProgressCallbackContext>);

impl Drop for UploadFileProgressCallbackContextWrapper {
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
            upload_file_progress_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for UploadFileProgressCallbackContextWrapper {}

pub type UploadFileResultCallback = extern "C" fn(
    status_code: u16,
    reason_phrase: *const c_char,
    xml_result: *const c_char,
    context: *mut UploadFileResultCallbackContext,
);

#[repr(C)]
pub struct UploadFileResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn upload_file_result_callback_context_release(context: *mut UploadFileResultCallbackContext);
}

pub struct UploadFileResultCallbackContextWrapper(pub NonNull<UploadFileResultCallbackContext>);

impl Drop for UploadFileResultCallbackContextWrapper {
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
            upload_file_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for UploadFileResultCallbackContextWrapper {}

pub type DownloadFileProgressCallback =
    extern "C" fn(current: u32, total: i32, context: *mut DownloadFileProgressCallbackContext);

#[repr(C)]
pub struct DownloadFileProgressCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn download_file_progress_callback_context_release(
        context: *mut DownloadFileProgressCallbackContext,
    );
}

pub struct DownloadileProgressCallbackContextWrapper(
    pub NonNull<DownloadFileProgressCallbackContext>,
);

impl Drop for DownloadileProgressCallbackContextWrapper {
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
            download_file_progress_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for DownloadileProgressCallbackContextWrapper {}

pub type DownloadFileResultCallback = extern "C" fn(
    status_code: u16,
    reason_phrase: *const c_char,
    context: *mut DownloadFileResultCallbackContext,
);

#[repr(C)]
pub struct DownloadFileResultCallbackContext {
    _data: [u8; 0],
    _marker: core::marker::PhantomData<(*mut u8, core::marker::PhantomPinned)>,
}

#[cfg(any(
    all(feature = "android", target_os = "android"),
    all(feature = "ohos", all(target_os = "linux", target_env = "ohos"))
))]
extern "C" {
    fn download_file_result_callback_context_release(
        context: *mut DownloadFileResultCallbackContext,
    );
}

pub struct DownloadFileResultCallbackContextWrapper(pub NonNull<DownloadFileResultCallbackContext>);

impl Drop for DownloadFileResultCallbackContextWrapper {
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
            download_file_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for DownloadFileResultCallbackContextWrapper {}
