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

use rust_rcs_core::{internet::HeaderField, util::raw_string::StrFind};

pub trait CPMAcceptContact<'a> {
    fn contains_icsi_ref(&self, icsi_ref: &[u8]) -> bool;
    fn contains_iari_ref(&self, iari_ref: &[u8]) -> bool;
}

impl<'a> CPMAcceptContact<'a> for HeaderField<'a> {
    fn contains_icsi_ref(&self, icsi_ref: &[u8]) -> bool {
        for p in self.get_parameter_iterator() {
            if p.name == b"+g.3gpp.icsi-ref" {
                if let Some(value) = p.value {
                    if let Some(_) = value.index_of(icsi_ref) {
                        return true;
                    }
                }
            }
        }

        false
    }

    fn contains_iari_ref(&self, iari_ref: &[u8]) -> bool {
        for p in self.get_parameter_iterator() {
            if p.name == b"+g.3gpp.iari-ref" {
                if let Some(value) = p.value {
                    if let Some(_) = value.index_of(iari_ref) {
                        return true;
                    }
                }
            }
        }

        false
    }
}
