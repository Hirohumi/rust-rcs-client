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

use rust_rcs_core::{
    dns::DnsConfig,
    ffi::net_ctrl::{get_active_network_info, get_dns_info, get_dns_servers, get_network_type},
};

use crate::provisioning::ims_application::{ImsApplication, MediaProtocol, TransportProto};

pub struct MsrpConnectionConfig {
    transport_proto: TransportProto,
}

impl MsrpConnectionConfig {
    pub fn new() -> MsrpConnectionConfig {
        MsrpConnectionConfig {
            transport_proto: TransportProto::create_default(),
        }
    }

    pub fn update_configuration(&mut self, ims_app: &ImsApplication) {
        let mut transport_proto = TransportProto::create_default();
        if let Some(gsma_ext) = ims_app.get_ims_gsma_extension() {
            if let Some(proto) = gsma_ext.get_info().get_transport_proto_info() {
                transport_proto = proto;
            }
        }

        self.transport_proto = transport_proto;
    }

    pub fn get_transport_config(&self) -> Option<(DnsConfig, bool)> {
        if let Some(network_info) = get_active_network_info() {
            let network_type = get_network_type(&network_info);

            let dns_info = get_dns_info(&network_info);

            if let Some(dns_info) = dns_info {
                let dns_servers = get_dns_servers(&dns_info);

                let dns_config = DnsConfig {
                    server_addrs: dns_servers,
                };

                let proto = if network_type == 1 {
                    &self.transport_proto.ps_media
                } else {
                    &self.transport_proto.wifi_media
                };

                return Some((
                    dns_config,
                    match proto {
                        MediaProtocol::MSRP => false,

                        MediaProtocol::MSRPoTLS => true,
                    },
                ));
            }
        }

        None
    }
}
