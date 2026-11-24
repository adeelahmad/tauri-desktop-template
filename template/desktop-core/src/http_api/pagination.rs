//! Pagination: `page` (from 1) and `perPage` (1 to [`MAX_PER_PAGE`]) in, a [`Paginated`]
//! list out with `hasMore`, so a client pages until `hasMore` is false.
//!
//! # Example
//!
//! ```
//! use desktop_core::http_api::Page;
//!
//! let page = Page { page: 2, per_page: 3 };
//! assert!(page.check().is_ok());
//! // Fetch one more than the end of the page, to know whether another page follows.
//! assert_eq!(page.fetch(), 7);
//! let out = page.paginate((1..=7).collect::<Vec<u32>>());
//! assert_eq!(out.items, [4, 5, 6]);
//! assert!(out.has_more);
//! ```

use super::FieldErrors;
use serde::{Deserialize, Serialize};

/// The largest `perPage`.
pub const MAX_PER_PAGE: u32 = 100;

/// The furthest a client can page into one list (the backend's own limit on results).
pub const MAX_ITEMS: usize = 1000;

/// Which page of a list to return.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase")]
pub struct Page {
    /// The page, from 1.
    pub page: u32,
    /// Items per page.
    pub per_page: u32,
}

impl Default for Page {
    fn default() -> Self {
        Self { page: 1, per_page: 20 }
    }
}

impl Page {
    /// Field errors for a page out of range.
    pub fn check(&self) -> Result<(), FieldErrors> {
        let mut e = FieldErrors::default();
        self.check_into(&mut e);
        e.into_result()
    }

    /// Add this page's problems to `e` (for requests with more fields).
    pub fn check_into(&self, e: &mut FieldErrors) {
        e.check(self.page >= 1, "page", "starts at 1");
        e.check((1..=MAX_PER_PAGE).contains(&self.per_page), "perPage", "must be between 1 and 100");
        e.check(self.end() <= MAX_ITEMS, "page", "goes past the first 1000 items");
    }

    /// Items to skip.
    pub fn offset(&self) -> usize {
        // Saturating: `page` and `perPage` come from the request, and usize is 32 bits on
        // some targets.
        (self.page.saturating_sub(1) as usize).saturating_mul(self.per_page as usize)
    }

    /// The index just past this page.
    pub fn end(&self) -> usize {
        self.offset().saturating_add(self.per_page as usize)
    }

    /// How many items to ask the backend for: the page and one more (to fill `hasMore`),
    /// at most [`MAX_ITEMS`].
    pub fn fetch(&self) -> usize {
        self.end().saturating_add(1).min(MAX_ITEMS)
    }

    /// This page of `all` (the first [`Page::fetch`] items of the full list).
    pub fn paginate<T>(&self, all: Vec<T>) -> Paginated<T> {
        let has_more = all.len() > self.end();
        let items = all.into_iter().skip(self.offset()).take(self.per_page as usize).collect();
        Paginated { items, page: self.page, per_page: self.per_page, has_more }
    }
}

/// One page of a list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Paginated<T> {
    /// The items on this page.
    pub items: Vec<T>,
    /// This page's number.
    pub page: u32,
    /// The page size asked for.
    pub per_page: u32,
    /// Another page follows.
    pub has_more: bool,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejects_pages_out_of_range() {
        assert!(Page { page: 0, per_page: 10 }.check().unwrap_err().get("page").len() == 1);
        assert!(!Page { page: 1, per_page: 101 }.check().unwrap_err().get("perPage").is_empty());
        assert!(Page { page: 11, per_page: 100 }.check().is_err());
        assert!(Page { page: 10, per_page: 100 }.check().is_ok());
    }

    #[test]
    fn last_page_has_no_more() {
        let p = Page { page: 2, per_page: 2 };
        let out = p.paginate(vec![1, 2, 3]);
        assert_eq!(out.items, [3]);
        assert!(!out.has_more);
    }

    #[test]
    fn huge_pages_do_not_overflow() {
        let p = Page { page: u32::MAX, per_page: u32::MAX };
        assert!(p.check().is_err());
        assert_eq!(p.fetch(), MAX_ITEMS);
        assert!(p.paginate(vec![1, 2, 3]).items.is_empty());
    }
}
