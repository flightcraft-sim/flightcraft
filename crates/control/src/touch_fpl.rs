//! Touch flight-plan editing on the landscape touch EFIS's FPL page: select a waypoint, then
//! Insert Before, Insert After, Remove, or Activate Leg (pilot's guide 190-01754-00 Rev. Y, 6.5
//! and 6.6, printed pages 191 to 193). Pure list arithmetic; the session applies the result to
//! the existing flight plan, so nothing here stores a plan of its own.
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// The longest flight plan the engine accepts (the guide allows 300; the engine's own limit is
/// the existing navigator's).
pub const MAX_WAYPOINTS: usize = 64;

/// One touch on the FPL page.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(tag = "kind", rename_all = "snake_case", deny_unknown_fields)]
pub enum FplEdit {
    /// Touch a waypoint in the list: it becomes the selected one.
    Select { index: usize },
    /// Insert Before or Insert After the selected waypoint, then enter its identifier. With no
    /// plan the waypoint becomes the whole plan.
    Insert {
        after: bool,
        #[schemars(length(min = 1, max = 16))]
        ident: String,
    },
    /// Remove the selected waypoint.
    Remove {},
    /// Activate Leg: the selected waypoint becomes the one flown to.
    Activate {},
}

/// Why an edit was refused.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FplError {
    Full,
    NothingSelected,
}

impl std::fmt::Display for FplError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Full => "flight plan is full",
            Self::NothingSelected => "select a waypoint in the flight plan first",
        })
    }
}

/// The result of an edit on a list of `len` waypoints.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Edited {
    /// Selected waypoint afterwards.
    pub cursor: usize,
    /// Active leg afterwards: the same waypoint stays active when the list shifts around it.
    pub active: usize,
}

/// Insert `item` before or after `cursor`. An empty list takes the item as its only waypoint.
pub fn insert<T>(
    items: &mut Vec<T>,
    cursor: usize,
    active: usize,
    after: bool,
    item: T,
) -> Result<Edited, FplError> {
    if items.len() >= MAX_WAYPOINTS {
        return Err(FplError::Full);
    }
    if items.is_empty() {
        items.push(item);
        return Ok(Edited { cursor: 0, active: 0 });
    }
    let cursor = cursor.min(items.len() - 1);
    let at = if after { cursor + 1 } else { cursor };
    items.insert(at, item);
    // Inserting at or before the active waypoint pushes it down one place; a new waypoint
    // inserted immediately before the active one is not itself active (the plan still goes to the
    // same waypoint next), as on the unit.
    let active = if at <= active { active + 1 } else { active };
    Ok(Edited { cursor: at, active })
}

/// Remove the waypoint at `cursor`.
pub fn remove<T>(items: &mut Vec<T>, cursor: usize, active: usize) -> Result<Edited, FplError> {
    if cursor >= items.len() {
        return Err(FplError::NothingSelected);
    }
    items.remove(cursor);
    if items.is_empty() {
        return Ok(Edited { cursor: 0, active: 0 });
    }
    let last = items.len() - 1;
    // Removing a waypoint before the active one keeps the same waypoint active; removing the
    // active one makes the next (or the last) active.
    let active = if cursor < active { active - 1 } else { active }.min(last);
    Ok(Edited {
        cursor: cursor.min(last),
        active,
    })
}

#[cfg(test)]
#[allow(clippy::unwrap_used)]
mod tests {
    use super::*;

    #[test]
    fn insert_before_and_after_keep_the_active_waypoint() {
        let mut v = vec!["A", "B", "C"];
        let e = insert(&mut v, 1, 1, false, "X").unwrap();
        assert_eq!(v, ["A", "X", "B", "C"]);
        assert_eq!((e.cursor, e.active), (1, 2));
        let e = insert(&mut v, 2, 2, true, "Y").unwrap();
        assert_eq!(v, ["A", "X", "B", "Y", "C"]);
        assert_eq!((e.cursor, e.active), (3, 2));
    }

    #[test]
    fn insert_into_an_empty_plan_and_past_the_limit() {
        let mut v: Vec<u32> = Vec::new();
        assert_eq!(
            insert(&mut v, 5, 0, true, 7).unwrap(),
            Edited { cursor: 0, active: 0 }
        );
        let mut full: Vec<usize> = (0..MAX_WAYPOINTS).collect();
        assert_eq!(insert(&mut full, 0, 0, true, 0), Err(FplError::Full));
    }

    #[test]
    fn remove_adjusts_cursor_and_active_leg() {
        let mut v = vec!["A", "B", "C", "D"];
        let e = remove(&mut v, 0, 2).unwrap();
        assert_eq!(v, ["B", "C", "D"]);
        assert_eq!((e.cursor, e.active), (0, 1));
        let e = remove(&mut v, 2, 2).unwrap();
        assert_eq!(v, ["B", "C"]);
        assert_eq!((e.cursor, e.active), (1, 1));
        assert_eq!(remove(&mut v, 9, 0), Err(FplError::NothingSelected));
        remove(&mut v, 0, 0).unwrap();
        assert_eq!(remove(&mut v, 0, 0).unwrap(), Edited { cursor: 0, active: 0 });
        assert!(v.is_empty());
    }
}
