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

extern crate rustls;
extern crate walkdir;

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::PathBuf;
use std::sync::Arc;
use std::{iter, panic};

use rustls::pki_types::CertificateDer;
use rustls::{ClientConfig, RootCertStore};
use rustls_pemfile::{read_one, Item};

use rust_rcs_core::dns::DnsClient;

use rust_rcs_core::ffi::log::platform_log;

use rust_rcs_core::http::HttpClient;

use rust_rcs_core::security::gba::GbaContext;
use rust_rcs_core::security::SecurityContext;

use rust_rcs_core::third_gen_pp::{addressing, CountryCode, NetworkCode};

use tokio::runtime::Runtime;
use tokio::sync::broadcast;
use walkdir::WalkDir;

const LOG_TAG: &str = "context";

pub struct Context {
    pub fs_root_dir: String,

    dns_client: Arc<DnsClient>,
    http_client: Arc<HttpClient>,

    tls_client_config: Arc<ClientConfig>,

    security_context: Arc<SecurityContext>,

    otp_broadcaster: broadcast::Sender<String>,
}

impl Context {
    pub fn new(fs_root_dir: &str, rt: Arc<Runtime>) -> Context {
        let tls_client_config = make_tls_client_config(fs_root_dir);
        let tls_client_config = Arc::new(tls_client_config);

        let dns_client = DnsClient::new(Arc::clone(&rt));
        let dns_client = Arc::new(dns_client);

        let http_client =
            HttpClient::new(Arc::clone(&tls_client_config), Arc::clone(&dns_client), rt);
        let http_client = Arc::new(http_client);

        let (otp_broadcaster, _) = broadcast::channel(1);

        Context {
            fs_root_dir: fs_root_dir.to_string(),

            dns_client,
            http_client,

            tls_client_config,

            security_context: Arc::new(SecurityContext::new()),

            otp_broadcaster,
        }
    }

    pub fn get_fs_root_dir(&self) -> &str {
        &self.fs_root_dir
    }

    pub fn get_dns_client(&self) -> Arc<DnsClient> {
        Arc::clone(&self.dns_client)
    }

    pub fn get_http_client(&self) -> Arc<HttpClient> {
        Arc::clone(&self.http_client)
    }

    pub fn get_tls_client_config(&self) -> Arc<ClientConfig> {
        Arc::clone(&self.tls_client_config)
    }

    pub fn get_security_context(&self) -> Arc<SecurityContext> {
        Arc::clone(&self.security_context)
    }

    pub fn subscribe_otp(&self) -> broadcast::Receiver<String> {
        self.otp_broadcaster.subscribe()
    }

    pub fn broadcast_otp(&self, otp: &str) {
        if let Err(e) = self.otp_broadcaster.send(String::from(otp)) {
            platform_log(LOG_TAG, format!("broadcast_otp failed with error {}", &e));
        }
    }

    pub fn make_gba_context(
        &self,
        imsi: &str,
        mcc: CountryCode,
        mnc: NetworkCode,
        subscription_id: i32,
    ) -> GbaContext {
        let impi = addressing::impi_from_imsi(imsi, mcc, mnc);

        let bsf_realm = addressing::bsf_address(mcc, mnc);
        let bsf_url = format! {"{}:8080", bsf_realm}; // port that actually works

        GbaContext::new(impi, bsf_url, bsf_realm, subscription_id)
    }
}

fn make_tls_client_config(root_dir: &str) -> ClientConfig {
    let mut root_certs = RootCertStore::empty();

    let mut path = PathBuf::from(root_dir);
    path.push("certs");

    platform_log(
        LOG_TAG,
        format!("searching for certificates under path {:?}", path),
    );

    for entry in WalkDir::new(path)
        .min_depth(1)
        .into_iter()
        .filter_entry(|e| -> bool {
            platform_log(LOG_TAG, format!("found entry {:?}", e));

            if let Some(file_name) = e.file_name().to_str() {
                file_name.ends_with(".der") || file_name.ends_with(".pem")
            } else {
                false
            }
        })
    {
        match entry {
            Ok(e) => {
                let path = e.path();
                platform_log(LOG_TAG, format!("init certificate with file {:?}", path));
                let mut f = File::open(path).unwrap();
                match path.extension() {
                    Some(ext) => {
                        if ext.eq_ignore_ascii_case("pem") {
                            let mut buf_reader = BufReader::new(f);
                            for item in iter::from_fn(|| read_one(&mut buf_reader).transpose()) {
                                match item {
                                    Ok(cert_item) => match cert_item {
                                        Item::X509Certificate(cert) => {
                                            platform_log(LOG_TAG, "adding X509Certificate");
                                            root_certs.add(cert).unwrap();
                                        }
                                        Item::Pkcs1Key(key) => platform_log(
                                            LOG_TAG,
                                            format!("rsa pkcs1 key {:?} not handled", key),
                                        ),
                                        Item::Pkcs8Key(key) => platform_log(
                                            LOG_TAG,
                                            format!("pkcs8 key {:?} not handled", key),
                                        ),
                                        Item::Sec1Key(key) => platform_log(
                                            LOG_TAG,
                                            format!("sec1 ec key {:?} not handled", key),
                                        ),
                                        _ => platform_log(LOG_TAG, "no certificate found"),
                                    },

                                    Err(e) => platform_log(LOG_TAG, format!("pem error {:?}", e)),
                                }
                            }
                        } else {
                            let mut v = vec![0; 4 * 1024];
                            f.read_to_end(&mut v).unwrap();

                            let cert = CertificateDer::from(v);
                            platform_log(LOG_TAG, format!("adding certificate {:?}", cert));
                            root_certs.add(cert).unwrap();
                        }
                    }
                    None => panic!(""),
                }
            }

            Err(e) => {}
        }
    }

    let client_config = ClientConfig::builder()
        .with_root_certificates(root_certs)
        .with_no_client_auth();

    client_config
}
