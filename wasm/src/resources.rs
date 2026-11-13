//! Resources JavaScript fetched ahead of a render. The renderer's resolver is synchronous
//! and `fetch` is not, so a render that asks for a resource not yet supplied records its URL
//! and is interrupted; the caller fetches what was recorded and renders again.

use std::cell::RefCell;
use std::collections::BTreeMap;

use serde::Serialize;

use ferri_pdf::{
    DataResolver, ResolvedResource, Resource, ResourceBase, ResourceError, ResourceMetadata,
    ResourceResolver, Url,
};

const NOT_FETCHED: &str = "not fetched yet";
const FILE_SCHEME: &str = "file";
const LOCAL_LOCATION: &str = "the response places a remote resource at a local file";

/// What a render asks JavaScript to fetch. A local file is asked for together with the
/// folder it must be in, so the same file answers separately for each folder.
#[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
pub(crate) struct Request {
    /// Without its fragment, which names a part of the resource and is not fetched.
    pub url: Url,
    /// The `file:` URL of the folder a local file is read in; absent for other URLs.
    pub file_root: Option<Url>,
}

/// A [`Request`] as JavaScript receives it.
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RequestReport {
    url: String,
    file_root: Option<String>,
}

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
    entries: RefCell<BTreeMap<Request, Entry>>,
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

    /// Starts a fetch round: what renders have asked for that is not yet answered.
    pub(crate) fn start_round(&mut self) -> Vec<RequestReport> {
        self.rounds += 1;
        self.entries
            .borrow()
            .iter()
            .filter(|(_, entry)| matches!(entry, Entry::Pending))
            .map(|(request, _)| RequestReport {
                url: request.url.as_str().to_owned(),
                file_root: request
                    .file_root
                    .as_ref()
                    .map(|root| root.as_str().to_owned()),
            })
            .collect()
    }

    /// The resources of a document that reads local files in `file_root`, if anywhere.
    pub(crate) fn confined(&self, file_root: Option<Url>) -> ConfinedResources<'_> {
        ConfinedResources {
            supplied: self,
            file_root,
        }
    }

    /// Bytes the job may still be supplied.
    pub(crate) fn remaining_bytes(&self) -> usize {
        self.limits.bytes - self.loaded_bytes
    }

    pub(crate) fn supply(
        &mut self,
        request: Request,
        bytes: Vec<u8>,
        metadata: ResourceMetadata,
        location: Option<Url>,
    ) {
        if bytes.len() > self.remaining_bytes() {
            let reason = self.byte_limit_reason();
            self.fail(request, &reason);
            return;
        }
        // As a redirect to a local file would, it lets remote content name local files.
        if request.file_root.is_none()
            && location
                .as_ref()
                .is_some_and(|location| location.scheme() == FILE_SCHEME)
        {
            self.fail(request, LOCAL_LOCATION);
            return;
        }
        self.loaded_bytes += bytes.len();
        let entry = Entry::Loaded {
            bytes,
            location: location.unwrap_or_else(|| request.url.clone()),
            metadata,
        };
        self.entries.get_mut().insert(request, entry);
    }

    pub(crate) fn fail(&mut self, request: Request, reason: &str) {
        self.entries
            .get_mut()
            .insert(request, Entry::Failed(reason.into()));
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

/// One document's view of the job's resources: it reads local files only in its own folder.
pub(crate) struct ConfinedResources<'a> {
    supplied: &'a SuppliedResources,
    file_root: Option<Url>,
}

impl ConfinedResources<'_> {
    /// The request for `url` made by content with `base`; a local file is refused here for
    /// remote content and for a document with no folder, and by JavaScript outside it.
    fn request(&self, url: Url, base: &ResourceBase) -> Result<Request, ResourceError> {
        if url.scheme() != FILE_SCHEME {
            return Ok(Request {
                url,
                file_root: None,
            });
        }
        if base.is_remote() {
            return Err(ResourceError::LocalFromRemote(url));
        }
        match &self.file_root {
            Some(root) => Ok(Request {
                url,
                file_root: Some(root.clone()),
            }),
            None => Err(ResourceError::NoFileRoot(url)),
        }
    }
}

impl ResourceResolver for ConfinedResources<'_> {
    fn resolve(&self, reference: &str, base: &ResourceBase) -> Result<Resource, ResourceError> {
        self.resolve_with_metadata(reference, base)
            .map(|resolved| resolved.resource)
    }

    fn resolve_with_metadata(
        &self,
        reference: &str,
        base: &ResourceBase,
    ) -> Result<ResolvedResource, ResourceError> {
        let supplied = self.supplied;
        if let Some(resolved) = supplied.data.resolve_data_url(reference, base) {
            return resolved;
        }
        let mut url = base.resolve(reference)?;
        url.set_fragment(None);
        let request = self.request(url, base)?;
        let unavailable = |reason: Box<str>| ResourceError::Unavailable {
            url: request.url.clone(),
            reason,
        };
        let mut entries = supplied.entries.borrow_mut();
        match entries.get(&request) {
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
            Some(Entry::Failed(reason)) => Err(unavailable(reason.clone())),
            Some(Entry::Pending) | None if supplied.exhausted() => {
                Err(unavailable(supplied.round_limit_reason().into()))
            }
            Some(Entry::Pending) | None => {
                let error = unavailable(NOT_FETCHED.into());
                entries.insert(request, Entry::Pending);
                Err(error)
            }
        }
    }

    fn interrupts(&self) -> bool {
        !self.supplied.exhausted()
            && self
                .supplied
                .entries
                .borrow()
                .values()
                .any(|entry| matches!(entry, Entry::Pending))
    }
}
