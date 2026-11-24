//! The Veyra DOM: an arena of nodes with tree mutations, serialisation,
//! selector parsing/matching and event primitives.
//!
//! The DOM is engine-agnostic: HTML construction, CSS styling, layout and
//! JavaScript bindings all operate on this structure.

pub mod event;
pub mod node;
pub mod selector;

pub use event::{Event, EventPhase, KeyEventData, MouseEventData};
pub use node::{Document, Node, NodeId, NodeType};
pub use selector::{matcher, matches_selector_list, Combinator, MatchContext, Selector, Specificity};
