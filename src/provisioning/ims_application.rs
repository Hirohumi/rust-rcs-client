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

use super::characteristic::{Characteristic, RuntimeCharacteristics};
use super::wap_provisioning_doc::WapProvisioningDoc;

pub const DEFAULT_TRANSPORT_PROTO: TransportProto = TransportProto {
    ps_signalling: SignallingProtocol::SIPoUDP,
    ps_media: MediaProtocol::MSRP,
    ps_rt_media: RealTimeMediaProtocol::RTP,

    ps_signalling_roaming: SignallingProtocol::SIPoUDP,
    ps_media_roaming: MediaProtocol::MSRP,
    ps_rt_media_roaming: RealTimeMediaProtocol::RTP,

    wifi_signalling: SignallingProtocol::SIPoTLS,
    wifi_media: MediaProtocol::MSRPoTLS,
    wifi_rt_media: RealTimeMediaProtocol::SRTP,
};

pub struct ImsApplication<'a> {
    root: &'a Characteristic,
}

impl<'a> ImsApplication<'a> {
    pub fn new(e: &Characteristic) -> ImsApplication {
        ImsApplication { root: e }
    }

    pub fn clone_characteristic(&self) -> Characteristic {
        self.root.clone()
    }

    pub fn get_ims_gsma_extension(&self) -> Option<ImsGMSAExtension> {
        if let Some(ext) = self.root.get_child_characteristic("Ext") {
            if let Some(gsma) = ext.get_child_characteristic("GSMA") {
                return Some(ImsGMSAExtension { root: gsma });
            }
        }

        None
    }

    pub fn get_lbo_p_cscf_addresses(&self) -> PCscfAddresses {
        if let Some(addresses) = self.root.get_child_characteristic("LBO_P-CSCF_Address") {
            let nodes = addresses.get_runtime_characteristics();
            return PCscfAddresses { root: Some(nodes) };
        }

        PCscfAddresses { root: None }
    }

    pub fn get_home_domain(&self) -> Option<&str> {
        if let Some(home) = self.root.get_parameter("Home_network_domain_name") {
            return Some(home);
        }

        None
    }

    pub fn get_private_user_identity(&self) -> Option<&str> {
        if let Some(pvui) = self.root.get_parameter("Private_User_Identity") {
            return Some(pvui);
        }

        None
    }

    pub fn get_public_user_identity_list(&self) -> IMPUList {
        if let Some(pbui_list) = self
            .root
            .get_child_characteristic("Public_User_Identity_List")
        {
            let nodes = pbui_list.get_runtime_characteristics();
            return IMPUList { root: Some(nodes) };
        }

        IMPUList { root: None }
    }
}

pub struct PCscfAddresses<'a> {
    root: Option<RuntimeCharacteristics<'a>>,
}

impl<'a> Iterator for PCscfAddresses<'a> {
    type Item = (&'a str, &'a str);
    fn next(&mut self) -> Option<(&'a str, &'a str)> {
        if let Some(ref mut nodes) = self.root {
            while let Some(node) = nodes.next() {
                if let (Some(address), Some(address_type)) = (
                    node.get_parameter("Address"),
                    node.get_parameter("AddressType"),
                ) {
                    return Some((address, address_type));
                }
            }
        }

        None
    }
}

pub struct IMPUList<'a> {
    root: Option<RuntimeCharacteristics<'a>>,
}

impl<'a> Iterator for IMPUList<'a> {
    type Item = &'a str;
    fn next(&mut self) -> Option<&'a str> {
        if let Some(ref mut nodes) = self.root {
            while let Some(node) = nodes.next() {
                if let Some(impu) = node.get_parameter("Public_User_Identity") {
                    return Some(impu);
                }
            }
        }

        None
    }
}

pub struct ImsGMSAExtension<'a> {
    root: &'a Characteristic,
}

impl<'a> ImsGMSAExtension<'a> {
    pub fn get_info(&self) -> ImsGSMAExtensionInfo {
        ImsGSMAExtensionInfo {
            app_ref: self.root.get_parameter("AppRef"),
            auth_type: self.root.get_parameter("AuthType"),
            realm: self.root.get_parameter("Realm"),
            user_name: self.root.get_parameter("UserName"),
            user_password: self.root.get_parameter("UserPwd"),
            eucr_id: self.root.get_parameter("endUserConfReqId"),
            transport_proto: self.root.get_child_characteristic("transportProto"),
            uuid_value: self.root.get_parameter("uuid_Value"),
        }
    }
}

pub struct ImsGSMAExtensionInfo<'a> {
    pub app_ref: Option<&'a str>,
    pub auth_type: Option<&'a str>,
    pub realm: Option<&'a str>,
    pub user_name: Option<&'a str>,
    pub user_password: Option<&'a str>,
    pub eucr_id: Option<&'a str>,
    transport_proto: Option<&'a Characteristic>,
    pub uuid_value: Option<&'a str>,
}

impl<'a> ImsGSMAExtensionInfo<'a> {
    pub fn get_transport_proto_info(&self) -> Option<TransportProto> {
        if let Some(e) = self.transport_proto {
            return Some(TransportProto {
                ps_signalling: if let Some(proto) = e.get_parameter("psSignalling") {
                    match proto {
                        "SIPoTCP" => SignallingProtocol::SIPoTCP,
                        "SIPoUDP" => SignallingProtocol::SIPoUDP,
                        "SIPoTLS" => SignallingProtocol::SIPoTLS,
                        _ => SignallingProtocol::SIPoUDP,
                    }
                } else {
                    SignallingProtocol::SIPoUDP
                },
                ps_media: if let Some(proto) = e.get_parameter("psMedia") {
                    match proto {
                        "MSRP" => MediaProtocol::MSRP,
                        "MSRPoTLS" => MediaProtocol::MSRPoTLS,
                        _ => MediaProtocol::MSRP,
                    }
                } else {
                    MediaProtocol::MSRP
                },
                ps_rt_media: if let Some(proto) = e.get_parameter("psRTMedia") {
                    match proto {
                        "RTP" => RealTimeMediaProtocol::RTP,
                        "SRTP" => RealTimeMediaProtocol::SRTP,
                        _ => RealTimeMediaProtocol::RTP,
                    }
                } else {
                    RealTimeMediaProtocol::RTP
                },
                ps_signalling_roaming: if let Some(proto) = e.get_parameter("psSignallingRoaming") {
                    match proto {
                        "SIPoTCP" => SignallingProtocol::SIPoTCP,
                        "SIPoUDP" => SignallingProtocol::SIPoUDP,
                        "SIPoTLS" => SignallingProtocol::SIPoTLS,
                        _ => SignallingProtocol::SIPoUDP,
                    }
                } else {
                    SignallingProtocol::SIPoUDP
                },
                ps_media_roaming: if let Some(proto) = e.get_parameter("psMediaRoaming") {
                    match proto {
                        "MSRP" => MediaProtocol::MSRP,
                        "MSRPoTLS" => MediaProtocol::MSRPoTLS,
                        _ => MediaProtocol::MSRP,
                    }
                } else {
                    MediaProtocol::MSRP
                },
                ps_rt_media_roaming: if let Some(proto) = e.get_parameter("psRTMediaRoaming") {
                    match proto {
                        "RTP" => RealTimeMediaProtocol::RTP,
                        "SRTP" => RealTimeMediaProtocol::SRTP,
                        _ => RealTimeMediaProtocol::RTP,
                    }
                } else {
                    RealTimeMediaProtocol::RTP
                },
                wifi_signalling: if let Some(proto) = e.get_parameter("wifiSignalling") {
                    match proto {
                        "SIPoTCP" => SignallingProtocol::SIPoTCP,
                        "SIPoUDP" => SignallingProtocol::SIPoUDP,
                        "SIPoTLS" => SignallingProtocol::SIPoTLS,
                        _ => SignallingProtocol::SIPoTLS,
                    }
                } else {
                    SignallingProtocol::SIPoTLS
                },
                wifi_media: if let Some(proto) = e.get_parameter("wifiMedia") {
                    match proto {
                        "MSRP" => MediaProtocol::MSRP,
                        "MSRPoTLS" => MediaProtocol::MSRPoTLS,
                        _ => MediaProtocol::MSRPoTLS,
                    }
                } else {
                    MediaProtocol::MSRPoTLS
                },
                wifi_rt_media: if let Some(proto) = e.get_parameter("wifiRTMedia") {
                    match proto {
                        "RTP" => RealTimeMediaProtocol::RTP,
                        "SRTP" => RealTimeMediaProtocol::SRTP,
                        _ => RealTimeMediaProtocol::SRTP,
                    }
                } else {
                    RealTimeMediaProtocol::SRTP
                },
            });
        }

        None
    }
}

pub enum SignallingProtocol {
    SIPoTCP,
    SIPoUDP,
    SIPoTLS,
}

pub enum MediaProtocol {
    MSRP,
    MSRPoTLS,
}

pub enum RealTimeMediaProtocol {
    RTP,
    SRTP,
}

pub struct TransportProto {
    pub ps_signalling: SignallingProtocol,
    pub ps_media: MediaProtocol,
    pub ps_rt_media: RealTimeMediaProtocol,

    pub ps_signalling_roaming: SignallingProtocol,
    pub ps_media_roaming: MediaProtocol,
    pub ps_rt_media_roaming: RealTimeMediaProtocol,

    pub wifi_signalling: SignallingProtocol,
    pub wifi_media: MediaProtocol,
    pub wifi_rt_media: RealTimeMediaProtocol,
}

impl TransportProto {
    pub fn create_default() -> TransportProto {
        TransportProto {
            ps_signalling: SignallingProtocol::SIPoUDP,
            ps_media: MediaProtocol::MSRP,
            ps_rt_media: RealTimeMediaProtocol::RTP,
            ps_signalling_roaming: SignallingProtocol::SIPoUDP,
            ps_media_roaming: MediaProtocol::MSRP,
            ps_rt_media_roaming: RealTimeMediaProtocol::RTP,
            wifi_signalling: SignallingProtocol::SIPoTLS,
            wifi_media: MediaProtocol::MSRPoTLS,
            wifi_rt_media: RealTimeMediaProtocol::SRTP,
        }
    }
}

pub trait GetImsApplication {
    fn get_ims_application(&self, app_ref: &str) -> Option<ImsApplication>;
}

impl<'a> GetImsApplication for &WapProvisioningDoc {
    fn get_ims_application(&self, app_ref: &str) -> Option<ImsApplication> {
        if let Some(ims_mo) = self.get_application_characteristic("urn:oma:mo:ext-3gpp-ims:1.0") {
            for child in &ims_mo.child_characteristics {
                if child.characteristic_type == "3GPP_IMS" {
                    if let Some(ext) = child.get_child_characteristic("Ext") {
                        if let Some(gsma) = ext.get_child_characteristic("GSMA") {
                            if let Some(app_ref_parm) = gsma.get_parameter("AppRef") {
                                if app_ref_parm == app_ref {
                                    return Some(ImsApplication { root: &child });
                                }
                            }
                        }
                    }
                }
            }
        }

        None
    }
}
