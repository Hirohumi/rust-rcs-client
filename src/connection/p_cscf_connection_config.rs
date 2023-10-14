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

use std::net::{IpAddr, Ipv4Addr};

use rust_rcs_core::{
    dns::DnsConfig,
    ffi::net_ctrl::{get_active_network_info, get_dns_info, get_dns_servers, get_network_type},
};

use crate::provisioning::ims_application::{ImsApplication, SignallingProtocol, TransportProto};

pub enum ServiceType {
    SipD2U,
    SipD2T,
    SipsD2T,
}

impl ServiceType {
    pub fn get_string_repr(&self) -> String {
        match self {
            ServiceType::SipD2U => String::from("SIP+D2U"),

            ServiceType::SipD2T => String::from("SIP+D2T"),

            ServiceType::SipsD2T => String::from("SIPS+D2T"),
        }
    }
}

pub struct PCscfConnectionConfig {
    p: usize,
    p_cscf_addresses: Vec<(String, String)>,
    transport_proto: TransportProto,
}

impl PCscfConnectionConfig {
    pub fn new() -> PCscfConnectionConfig {
        PCscfConnectionConfig {
            p: 0,
            p_cscf_addresses: Vec::new(),
            transport_proto: TransportProto::create_default(),
        }
    }

    pub fn update_configuration(&mut self, ims_app: &ImsApplication) {
        let mut p_cscf_addresses = Vec::new();

        for (address, address_type) in ims_app.get_lbo_p_cscf_addresses() {
            p_cscf_addresses.push((String::from(address), String::from(address_type)));
        }

        let mut transport_proto = TransportProto::create_default();
        if let Some(gsma_ext) = ims_app.get_ims_gsma_extension() {
            if let Some(proto) = gsma_ext.get_info().get_transport_proto_info() {
                transport_proto = proto;
            }
        }

        self.p = 0;
        self.p_cscf_addresses = p_cscf_addresses;
        self.transport_proto = transport_proto;
    }

    pub fn get_next(
        &mut self,
    ) -> Option<(
        DnsConfig,
        ServiceType,
        String,
        Option<String>,
        Option<IpAddr>,
        Option<u16>,
    )> {
        if let Some(network_info) = get_active_network_info() {
            let network_type = get_network_type(&network_info);

            let dns_info = get_dns_info(&network_info);

            if let Some(dns_info) = dns_info {
                let dns_servers = get_dns_servers(&dns_info);

                let dns_config = DnsConfig {
                    server_addrs: dns_servers,
                };

                if self.p_cscf_addresses.len() > 0 {
                    self.p = if self.p_cscf_addresses.len() > 1 {
                        self.p % self.p_cscf_addresses.len()
                    } else {
                        0
                    };
                    let p_cscf_address = self.p_cscf_addresses.get(self.p);
                    self.p += 1;
                    if let Some((address, address_type)) = p_cscf_address {
                        let proto = if network_type == 2 {
                            &self.transport_proto.ps_signalling
                        } else if network_type == 3 {
                            &self.transport_proto.ps_signalling_roaming
                        } else {
                            &self.transport_proto.wifi_signalling
                        };

                        let mut host: Option<String> = None;
                        let mut addr: Option<IpAddr> = None;
                        let mut port: Option<u16> = None;
                        if address_type == "FQDN" {
                            host = Some(String::from(address));
                        } else if address_type == "IPv4" {
                            if let Some(idx) = address.find(':') {
                                let v4_addr = &address[..idx];
                                let v4_port = &address[idx + 1..];
                                let v4_addr: Result<Ipv4Addr, _> = v4_addr.parse();
                                if let Ok(v4_addr) = v4_addr {
                                    addr = Some(IpAddr::V4(v4_addr));
                                } else {
                                    return self.get_next();
                                }
                                let v4_port: Result<u16, _> = v4_port.parse();
                                if let Ok(v4_port) = v4_port {
                                    port = Some(v4_port);
                                } else {
                                    return self.get_next();
                                }
                            } else {
                                let v4_addr: Result<Ipv4Addr, _> = address.parse();
                                if let Ok(v4_addr) = v4_addr {
                                    addr = Some(IpAddr::V4(v4_addr));
                                } else {
                                    return self.get_next();
                                }
                            }
                        } else {
                            return self.get_next();
                        }

                        let service_type = match proto {
                            SignallingProtocol::SIPoUDP => ServiceType::SipD2U,

                            SignallingProtocol::SIPoTCP => ServiceType::SipD2T,

                            SignallingProtocol::SIPoTLS => ServiceType::SipsD2T,
                        };

                        return Some((
                            dns_config,
                            service_type,
                            String::from(address),
                            host,
                            addr,
                            port,
                        ));
                    }
                }
            }
        }

        None
    }
}
