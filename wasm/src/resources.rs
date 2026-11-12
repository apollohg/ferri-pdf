//! Resources JavaScript fetched ahead of a render. The renderer's resolver is synchronous
//! and `fetch` is not, so a render that asks for a resource not yet supplied records its URL
//! and is interrupted; the caller fetches what was recorded and renders again.

use std::cell::RefCell;
use std::collections::BTreeMap;

use ferri_pdf::{
    DataResolver, ResolvedResource, Resource, ResourceBase, ResourceError, ResourceMetadata,
    ResourceResolver, Url,
};

const NOT_FETCHED: &str = "not fetched yet";

/// The bounds on what one render job fetches.
#[derive(Clone, Copy)]
pub(crate) struct FetchLimits {
    /// Times the caller is asked to fetch the resources a render lacked.
    pub rounds: u32,
    /// Bytes of all supplied resources together.
    pub bytes: usize,
}

enum Entry {
    /// Asked for by a render and not yet answered.
    Pending,
    Loaded {
        bytes: Vec<u8>,
        /// Where the resource ended up after redirects, for its relative URLs.
        location: Url,
        metadata: ResourceMetadata,
    },
    Failed(Box<str>),
}

pub(crate) struct SuppliedResources {
    /// By URL without its fragment, which names a part of the resource and is not fetched.
    entries: RefCell<BTreeMap<Url, Entry>>,
    data: DataResolver,
    limits: FetchLimits,
    rounds: u32,
    loaded_bytes: usize,
}

impl SuppliedResources {
    pub(crate) fn new(limits: FetchLimits) -> Self {
        Self {
            entries: RefCell::default(),
            data: DataResolver::default(),
            limits,
            rounds: 0,
            loaded_bytes: 0,
        }
    }

    /// Whether the caller will not be asked to fetch again, so what is still missing stays
    /// unavailable.
    fn exhausted(&self) -> bool {
        self.rounds >= self.limits.rounds
    }

    /// Starts a fetch round: the URLs renders have asked for that are not yet answered.
    pub(crate) fn start_round(&mut self) -> Vec<String> {
        self.rounds += 1;
        self.entries
            .borrow()
            .iter()
            .filter(|(_, entry)| matches!(entry, Entry::Pending))
            .map(|(url, _)| url.as_str().to_owned())
            .collect()
    }

    /// Bytes the job may still be supplied.
    pub(crate) fn remaining_bytes(&self) -> usize {
        self.limits.bytes - self.loaded_bytes
    }

    pub(crate) fn supply(
        &mut self,
        url: Url,
        bytes: Vec<u8>,
        metadata: ResourceMetadata,
        location: Option<Url>,
    ) {
        if bytes.len() > self.remaining_bytes() {
            let reason = self.byte_limit_reason();
            self.fail(url, &reason);
            return;
        }
        self.loaded_bytes += bytes.len();
        let entry = Entry::Loaded {
            bytes,
            location: location.unwrap_or_else(|| url.clone()),
            metadata,
        };
        self.entries.get_mut().insert(url, entry);
    }

    pub(crate) fn fail(&mut self, url: Url, reason: &str) {
        self.entries
            .get_mut()
            .insert(url, Entry::Failed(reason.into()));
    }

    pub(crate) fn byte_limit_reason(&self) -> String {
        format!(
            "the job's limit of {} fetched bytes was reached",
            self.limits.bytes
        )
    }

    fn round_limit_reason(&self) -> String {
        format!(
            "the job's limit of {} fetch rounds was reached",
            self.limits.rounds
        )
    }
}

fn unavailable(url: Url, reason: impl Into<Box<str>>) -> ResourceError {
    ResourceError::Unavailable {
        url,
        reason: reason.into(),
    }
}

impl ResourceResolver for SuppliedResources {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.resolve_with_metadata(reference, base)
            .map(|resolved| resolved.resource)
    }

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        if let Some(resolved) = self.data.resolve_data_url(reference, base) {
            return resolved;
        }
        let mut url = base.resolve(reference)?;
        url.set_fragment(None);
        let mut entries = self.entries.borrow_mut();
        match entries.get(&url) {
            Some(Entry::Loaded {
                bytes,
                location,
                metadata,
            }) => Ok(ResolvedResource {
                resource: Resource {
                    bytes: bytes.clone(),
                    base: ResourceBase::Url(location.clone()),
                },
                metadata: metadata.clone(),
            }),
            Some(Entry::Failed(reason)) => Err(unavailable(url, reason.clone())),
            Some(Entry::Pending) | None if self.exhausted() => {
                Err(unavailable(url, self.round_limit_reason()))
            }
            Some(Entry::Pending) | None => {
                entries.insert(url.clone(), Entry::Pending);
                Err(unavailable(url, NOT_FETCHED))
            }
        }
    }

    fn interrupts(&self) -> bool {
        !self.exhausted()
            && self
                .entries
                .borrow()
                .values()
                .any(|entry| matches!(entry, Entry::Pending))
    }
}
