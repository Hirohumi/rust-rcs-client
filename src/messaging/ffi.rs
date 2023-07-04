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

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
extern "C" {
    fn message_result_callback_context_release(context: *mut MessageResultCallbackContext);
}

pub struct MessageResultCallbackContextWrapper(pub NonNull<MessageResultCallbackContext>);

impl Drop for MessageResultCallbackContextWrapper {
    fn drop(&mut self) {
        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        let cb_context = self.0.as_ptr();
        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
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

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
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
        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        let cb_context = self.0.as_ptr();
        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        unsafe {
            send_imdn_report_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for SendImdnReportResultCallbackContextWrapper {}

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

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
extern "C" {
    fn upload_file_result_callback_context_release(context: *mut UploadFileResultCallbackContext);
}

pub struct UploadFileResultCallbackContextWrapper(pub NonNull<UploadFileResultCallbackContext>);

impl Drop for UploadFileResultCallbackContextWrapper {
    fn drop(&mut self) {
        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        let cb_context = self.0.as_ptr();
        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        unsafe {
            upload_file_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for UploadFileResultCallbackContextWrapper {}

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

#[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
extern "C" {
    fn download_file_result_callback_context_release(
        context: *mut DownloadFileResultCallbackContext,
    );
}

pub struct DownloadFileResultCallbackContextWrapper(pub NonNull<DownloadFileResultCallbackContext>);

impl Drop for DownloadFileResultCallbackContextWrapper {
    fn drop(&mut self) {
        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        let cb_context = self.0.as_ptr();
        #[cfg(any(target_arch = "arm", target_arch = "aarch64"))]
        unsafe {
            download_file_result_callback_context_release(cb_context);
        }
    }
}

unsafe impl Send for DownloadFileResultCallbackContextWrapper {}
