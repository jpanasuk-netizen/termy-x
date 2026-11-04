//! Bounded interning for OSC 8 hyperlinks.
//!
//! Cells share one `Arc<Hyperlink>` per `(id, uri)`, and the table caps the
//! bytes of distinct links that cells may reference. Without it, every OSC 8
//! allocated a fresh URI, so scrollback's cell bound did not bound memory.
use super::types::Hyperlink;
use std::{
    borrow::Borrow,
    collections::HashSet,
    hash::{Hash, Hasher},
    sync::Arc,
};

/// Longest URI accepted; longer links print as plain text (kitty uses 2 KiB).
const MAX_URI_BYTES: usize = 8 * 1024;
const MAX_ID_BYTES: usize = 256;
/// Bytes of distinct link text retained across the screen and scrollback.
const MAX_TABLE_BYTES: usize = 4 * 1024 * 1024;
const MAX_TABLE_LINKS: usize = 1 << 16;
/// A full table is rescanned only once new-link requests since the last
/// rescan reach this fraction of its size, so rescans cost amortized
/// O(`RESCAN_DIVISOR`) per OSC 8 however few links each one frees.
const RESCAN_DIVISOR: usize = 16;

/// An `(id, uri)` view, so lookups borrow the request instead of building an
/// owned key. An interned link is its own key and holds the only copy.
trait LinkKey {
    fn parts(&self) -> (&str, &str);
}

impl LinkKey for (&str, &str) {
    fn parts(&self) -> (&str, &str) {
        *self
    }
}

impl Hash for dyn LinkKey + '_ {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.parts().hash(state);
    }
}

impl PartialEq for dyn LinkKey + '_ {
    fn eq(&self, other: &Self) -> bool {
        self.parts() == other.parts()
    }
}

impl Eq for dyn LinkKey + '_ {}

struct Interned(Arc<Hyperlink>);

impl LinkKey for Interned {
    fn parts(&self) -> (&str, &str) {
        (&self.0.id, &self.0.uri)
    }
}

impl<'a> Borrow<dyn LinkKey + 'a> for Interned {
    fn borrow(&self) -> &(dyn LinkKey + 'a) {
        self
    }
}

impl Hash for Interned {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (self as &dyn LinkKey).hash(state);
    }
}

impl PartialEq for Interned {
    fn eq(&self, other: &Self) -> bool {
        self.parts() == other.parts()
    }
}

impl Eq for Interned {}

#[derive(Default)]
pub(super) struct Hyperlinks {
    links: HashSet<Interned>,
    bytes: usize,
    // New-link requests since the last rescan.
    requests: usize,
    #[cfg(test)]
    rescans: usize,
}

impl Hyperlinks {
    /// Every interned link, referenced by cells or not, for snapshots.
    pub(super) fn links(&self) -> impl Iterator<Item = &Arc<Hyperlink>> {
        self.links.iter().map(|link| &link.0)
    }

    pub(super) fn requests(&self) -> usize {
        self.requests
    }

    /// A table holding exactly `links`, which share identity with the
    /// cells a snapshot restores.
    pub(super) fn restore(
        links: impl IntoIterator<Item = Arc<Hyperlink>>,
        requests: usize,
    ) -> Self {
        let mut table = Self {
            requests,
            ..Self::default()
        };
        for link in links {
            table.bytes += link.id.len() + link.uri.len();
            table.links.insert(Interned(link));
        }
        table
    }

    /// The shared link for `(id, uri)`, or `None` when it is oversized or the
    /// table is full of links that cells still reference.
    pub(super) fn intern(&mut self, id: &str, uri: &str) -> Option<Arc<Hyperlink>> {
        if uri.len() > MAX_URI_BYTES || id.len() > MAX_ID_BYTES {
            return None;
        }
        if let Some(link) = self.links.get(&(id, uri) as &dyn LinkKey) {
            return Some(link.0.clone());
        }
        self.requests = self.requests.saturating_add(1);
        let size = id.len() + uri.len();
        let full = |links: &Self| {
            links.bytes + size > MAX_TABLE_BYTES || links.links.len() >= MAX_TABLE_LINKS
        };
        if full(self) {
            if self.requests < self.links.len() / RESCAN_DIVISOR {
                return None;
            }
            self.requests = 0;
            #[cfg(test)]
            {
                self.rescans += 1;
            }
            // Links held only by this table no longer appear in any cell.
            let bytes = &mut self.bytes;
            self.links.retain(|link| {
                let referenced = Arc::strong_count(&link.0) > 1;
                if !referenced {
                    *bytes -= link.0.id.len() + link.0.uri.len();
                }
                referenced
            });
            if full(self) {
                return None;
            }
        }
        let link = Arc::new(Hyperlink {
            id: id.to_owned(),
            uri: uri.to_owned(),
        });
        self.bytes += size;
        self.links.insert(Interned(link.clone()));
        Some(link)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn identical_links_share_one_allocation() {
        let mut links = Hyperlinks::default();
        let first = links.intern("a", "https://example.test").unwrap();
        let second = links.intern("a", "https://example.test").unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        assert!(!Arc::ptr_eq(
            &first,
            &links.intern("b", "https://example.test").unwrap()
        ));
        // The id/uri split is part of the key, not just their concatenation.
        assert!(!Arc::ptr_eq(
            &links.intern("ab", "c").unwrap(),
            &links.intern("a", "bc").unwrap()
        ));
        assert!(links.intern("", &"x".repeat(MAX_URI_BYTES + 1)).is_none());
        assert_eq!(links.bytes, "ahttps://example.test".len() * 2 + 6);
    }

    #[test]
    fn referenced_links_are_bounded_and_unreferenced_ones_are_reclaimed() {
        let mut links = Hyperlinks::default();
        let uri = "u".repeat(MAX_URI_BYTES);
        let held: Vec<_> = (0..)
            .map_while(|index| links.intern(&index.to_string(), &uri))
            .collect();
        assert!(held.len() * MAX_URI_BYTES <= MAX_TABLE_BYTES);
        assert!(links.intern("next", &uri).is_none());
        drop(held);
        // Released links are reclaimed by a rescan within a bounded number
        // of requests.
        let interval = links.links.len() / RESCAN_DIVISOR;
        let rejected = (0..=interval)
            .take_while(|_| links.intern("next", &uri).is_none())
            .count();
        assert!(rejected <= interval);
        assert_eq!(links.bytes, "next".len() + MAX_URI_BYTES);
    }

    #[test]
    fn full_table_rescans_are_amortized_and_never_evict_referenced_links() {
        // A table full of referenced links where each new link frees only one
        // old one, like scrollback evicting a linked row per line.
        let mut links = Hyperlinks::default();
        let mut held: std::collections::VecDeque<_> = (0..MAX_TABLE_LINKS)
            .map(|index| links.intern("", &index.to_string()).unwrap())
            .collect();
        let rescans = links.rescans;
        let requests = 4 * MAX_TABLE_LINKS;
        let mut accepted = 0;
        for index in MAX_TABLE_LINKS..MAX_TABLE_LINKS + requests {
            held.pop_front();
            if let Some(link) = links.intern("", &index.to_string()) {
                held.push_back(link);
                accepted += 1;
            }
        }
        // Each rescan reads the whole table, so it must be paid for by a
        // proportional number of requests.
        assert!(links.rescans - rescans <= requests * RESCAN_DIVISOR / MAX_TABLE_LINKS + 1);
        // Rejections are bounded by the rescan interval, not by the table.
        assert!(accepted >= requests / 2);
        // A referenced link is never evicted: interning it again returns the
        // very same allocation, and the table is still within budget.
        for link in &held {
            assert!(Arc::ptr_eq(
                link,
                &links.intern(&link.id, &link.uri).unwrap()
            ));
        }
        assert!(links.links.len() <= MAX_TABLE_LINKS);
    }
}
