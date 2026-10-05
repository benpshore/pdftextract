//! Small helpers over `roxmltree` that match elements and attributes by local
//! name, so OOXML prefixes (`w:`, `a:`, `p:`, `r:`) need no namespace table.

use roxmltree::Node;

/// Whether `node` is an element with local name `name`.
pub fn is(node: Node<'_, '_>, name: &str) -> bool {
    node.is_element() && node.tag_name().name() == name
}

/// The value of the attribute with local name `name`, in any namespace.
pub fn attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == name)
        .map(|a| a.value())
}

/// OOXML relationships namespace (`r:` attributes).
pub const RELATIONSHIPS_NS: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships";

/// The `r:<name>` attribute (relationships namespace), falling back to any
/// namespaced attribute with that local name. Plain `id` never matches, so
/// `<p:sldId id="256" r:id="rId2"/>` yields `rId2`.
pub fn rel_attr<'a>(node: Node<'a, '_>, name: &str) -> Option<&'a str> {
    node.attributes()
        .find(|a| a.name() == name && a.namespace() == Some(RELATIONSHIPS_NS))
        .or_else(|| {
            node.attributes()
                .find(|a| a.name() == name && a.namespace().is_some())
        })
        .map(|a| a.value())
}

/// The first child element with local name `name`.
pub fn child<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.children().find(|c| is(*c, name))
}

/// All child elements with local name `name`, in document order.
pub fn children<'a, 'i>(node: Node<'a, 'i>, name: &str) -> impl Iterator<Item = Node<'a, 'i>> {
    let name = name.to_string();
    node.children().filter(move |c| is(*c, &name))
}

/// The first descendant element with local name `name`.
pub fn descendant<'a, 'i>(node: Node<'a, 'i>, name: &str) -> Option<Node<'a, 'i>> {
    node.descendants().find(|c| is(*c, name))
}

/// The concatenated text of all text nodes below `node`.
pub fn text_of(node: Node<'_, '_>) -> String {
    let mut out = String::new();
    for d in node.descendants() {
        if d.is_text() {
            out.push_str(d.text().unwrap_or(""));
        }
    }
    out
}

/// Parse an XML part, mapping the error to the part name.
pub fn parse<'a>(
    name: &str,
    source: &'a str,
) -> Result<roxmltree::Document<'a>, crate::FormatsError> {
    let options = roxmltree::ParsingOptions {
        allow_dtd: false,
        nodes_limit: u32::MAX,
    };
    roxmltree::Document::parse_with_options(source, options)
        .map_err(|e| crate::FormatsError::Xml(format!("{name}: {e}")))
}
