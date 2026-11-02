//! The Mediathek's filter menu: which of the timeline's photos are shown. Pure, so it is unit-tested.

use super::models::timeline::TimelineItem;
use std::collections::BTreeSet;

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub enum Kind {
    #[default]
    All,
    Photos,
    Videos,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Debug)]
pub struct Filter {
    /// Only photos that are in no album.
    pub no_album: bool,
    pub favorites: bool,
    /// Only photos stored on this device.
    pub downloaded: bool,
    pub kind: Kind,
}

impl Filter {
    pub fn is_active(&self) -> bool {
        *self != Self::default()
    }

    /// Whether the item passes the filters that look at the item itself (everything but `no_album`, which needs the album lookup).
    fn keeps(&self, item: &TimelineItem, is_downloaded: &impl Fn(&str) -> bool) -> bool {
        (!self.favorites || item.is_favorite)
            && (!self.downloaded || is_downloaded(&item.id))
            && match self.kind {
                Kind::All => true,
                Kind::Photos => item.is_image(),
                Kind::Videos => item.is_video(),
            }
    }

    /// The items that pass, in their order. `in_albums` says which of the ids belong to an album; it is only asked
    /// (one pass over the ids) when `no_album` is set.
    pub fn apply(&self, items: &[TimelineItem], in_albums: impl FnOnce(&[String]) -> BTreeSet<String>, is_downloaded: impl Fn(&str) -> bool) -> Vec<TimelineItem> {
        let linked = self.no_album.then(|| {
            let ids: Vec<String> = items.iter().filter(|i| self.keeps(i, &is_downloaded)).map(|i| i.id.clone()).collect();
            in_albums(&ids)
        });
        items.iter().filter(|i| self.keeps(i, &is_downloaded) && linked.as_ref().is_none_or(|l| !l.contains(&i.id))).cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, asset_type: &str, favorite: bool) -> TimelineItem {
        TimelineItem {
            id: id.into(),
            asset_type: asset_type.into(),
            thumbhash: None,
            ratio: None,
            is_favorite: favorite,
            is_trashed: false,
            date_bucket: String::new(),
            sort_date: None,
            duration: None,
        }
    }

    fn ids(items: &[TimelineItem]) -> Vec<&str> {
        items.iter().map(|i| i.id.as_str()).collect()
    }

    fn albums(of: &[&str]) -> impl FnOnce(&[String]) -> BTreeSet<String> {
        let of: BTreeSet<String> = of.iter().map(|s| s.to_string()).collect();
        move |ids: &[String]| ids.iter().filter(|i| of.contains(*i)).cloned().collect()
    }

    #[::core::prelude::v1::test]
    fn default_keeps_everything() {
        let items = [item("a", "IMAGE", false), item("b", "VIDEO", true)];
        assert!(!Filter::default().is_active());
        assert_eq!(ids(&Filter::default().apply(&items, albums(&[]), |_| false)), ["a", "b"]);
    }

    #[::core::prelude::v1::test]
    fn no_album_drops_linked_photos_and_keeps_order() {
        let items = [item("a", "IMAGE", false), item("b", "IMAGE", false), item("c", "VIDEO", false)];
        let f = Filter { no_album: true, ..Default::default() };
        assert!(f.is_active());
        assert_eq!(ids(&f.apply(&items, albums(&["b"]), |_| false)), ["a", "c"]);
    }

    #[::core::prelude::v1::test]
    fn filters_combine() {
        let items = [item("a", "IMAGE", true), item("b", "VIDEO", true), item("c", "IMAGE", false), item("d", "IMAGE", true)];
        let f = Filter { no_album: true, favorites: true, kind: Kind::Photos, ..Default::default() };
        assert_eq!(ids(&f.apply(&items, albums(&["d"]), |_| false)), ["a"]);
        let f = Filter { kind: Kind::Videos, ..Default::default() };
        assert_eq!(ids(&f.apply(&items, albums(&[]), |_| false)), ["b"]);
    }

    #[::core::prelude::v1::test]
    fn downloaded_keeps_only_stored_ids() {
        let items = [item("a", "IMAGE", false), item("b", "VIDEO", false)];
        let f = Filter { downloaded: true, ..Default::default() };
        assert!(f.is_active());
        assert_eq!(ids(&f.apply(&items, albums(&[]), |id| id == "b")), ["b"]);
    }

    #[::core::prelude::v1::test]
    fn album_lookup_only_runs_for_no_album() {
        let items = [item("a", "IMAGE", false)];
        let f = Filter { favorites: true, ..Default::default() };
        assert!(f.apply(&items, |_| panic!("not needed"), |_| false).is_empty());
    }
}
