use crate::{innerlude::AttributeValue, AnyEvent, ElementId, VNode};
use bumpalo::boxed::Box as BumpBox;
use std::{
    cell::{Cell, RefCell},
    fmt::{Debug, Formatter},
};

/// An element like a "div" with children, listeners, and attributes.
pub struct VElement<'a> {
    /// The [`ElementId`] of the VText.
    pub id: Cell<Option<ElementId>>,

    /// The key of the element to be used during keyed diffing.
    pub key: Option<&'a str>,

    /// The tag name of the element.
    ///
    /// IE "div"
    pub tag: &'static str,

    /// The namespace of the VElement
    ///
    /// IE "svg"
    pub namespace: Option<&'static str>,

    /// The parent of the Element (if any).
    ///
    /// Used when bubbling events
    pub parent: Cell<Option<ElementId>>,

    /// The Listeners of the VElement.
    pub listeners: &'a [Listener<'a>],

    /// The attributes of the VElement.
    pub attributes: &'a [Attribute<'a>],

    /// The children of the VElement.
    pub children: &'a [VNode<'a>],
}

impl Debug for VElement<'_> {
    fn fmt(&self, f: &mut Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("VElement")
            .field("tag_name", &self.tag)
            .field("namespace", &self.namespace)
            .field("key", &self.key)
            .field("id", &self.id)
            .field("parent", &self.parent)
            .field("listeners", &self.listeners.len())
            .field("attributes", &self.attributes)
            .field("children", &self.children)
            .finish()
    }
}

/// An attribute on a DOM node, such as `id="my-thing"` or
/// `href="https://example.com"`.
#[derive(Clone, Debug)]
pub struct Attribute<'a> {
    /// The name of the attribute.
    pub name: &'static str,

    /// The namespace of the attribute.
    ///
    /// Doesn't exist in the html spec.
    /// Used in Dioxus to denote "style" tags and other attribute groups.
    pub namespace: Option<&'static str>,

    /// An indication of we should always try and set the attribute.
    /// Used in controlled components to ensure changes are propagated.
    pub volatile: bool,

    /// The value of the attribute.
    pub value: AttributeValue<'a>,
}

/// An event listener.
/// IE onclick, onkeydown, etc
pub struct Listener<'bump> {
    /// The ID of the node that this listener is mounted to
    /// Used to generate the event listener's ID on the DOM
    pub mounted_node: Cell<Option<ElementId>>,

    /// The type of event to listen for.
    ///
    /// IE "click" - whatever the renderer needs to attach the listener by name.
    pub event: &'static str,

    /// The actual callback that the user specified
    pub(crate) callback: InternalHandler<'bump>,
}

pub type InternalHandler<'bump> = &'bump RefCell<Option<InternalListenerCallback<'bump>>>;
type InternalListenerCallback<'bump> = BumpBox<'bump, dyn FnMut(AnyEvent) + 'bump>;
type ExternalListenerCallback<'bump, T> = BumpBox<'bump, dyn FnMut(T) + 'bump>;

/// The callback type generated by the `rsx!` macro when an `on` field is specified for components.
///
/// This makes it possible to pass `move |evt| {}` style closures into components as property fields.
///
///
/// # Example
///
/// ```rust, ignore
///
/// rsx!{
///     MyComponent { onclick: move |evt| log::info!("clicked"), }
/// }
///
/// #[derive(Props)]
/// struct MyProps<'a> {
///     onclick: EventHandler<'a, MouseEvent>,
/// }
///
/// fn MyComponent(cx: Scope<'a, MyProps<'a>>) -> Element {
///     cx.render(rsx!{
///         button {
///             onclick: move |evt| cx.props.onclick.call(evt),
///         }
///     })
/// }
///
/// ```
pub struct EventHandler<'bump, T = ()> {
    /// The (optional) callback that the user specified
    /// Uses a `RefCell` to allow for interior mutability, and FnMut closures.
    pub callback: RefCell<Option<ExternalListenerCallback<'bump, T>>>,
}

impl<'a, T> Default for EventHandler<'a, T> {
    fn default() -> Self {
        Self {
            callback: RefCell::new(None),
        }
    }
}

impl<T> EventHandler<'_, T> {
    /// Call this event handler with the appropriate event type
    pub fn call(&self, event: T) {
        if let Some(callback) = self.callback.borrow_mut().as_mut() {
            callback(event);
        }
    }

    /// Forcibly drop the internal handler callback, releasing memory
    pub fn release(&self) {
        self.callback.replace(None);
    }
}
