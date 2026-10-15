//! Zotero RDF: the RDF/XML that Zotero's own "Zotero RDF" export writes.
//!
//! The statements follow the translator `Zotero RDF.js` (translator id
//! `14763d24-8ba0-45df-8f52-b8d1108e7ac9`): typed `bib:` nodes with
//! `z:itemType`, containers under `dcterms:isPartOf`, creators as
//! `foaf:Person` in an `rdf:Seq`, identifiers as `dc:identifier` literals
//! (`DOI …`, `ISSN …`, `ISBN …`) or a `dcterms:URI` node, and `dc:relation`
//! between related items. The text layout (namespace declarations one per
//! line, four-space indentation, short subtrees folded onto one line)
//! reproduces the RDF/XML serializer Zotero bundles
//! (`translate/src/rdf/serialize.js`, `statementsToXML`).
//!
//! Two statements are added for reverse citation: every cited work carries
//! `dcterms:isReferencedBy` pointing at the citing article, and the article
//! and each cited work are related in both directions with `dc:relation`,
//! which Zotero imports as "Related" items. Zotero's importer only turns a
//! `dcterms:isReferencedBy` target into a child note when that target is a
//! `bib:Memo`, so the link is kept on import without side effects.

use std::collections::BTreeSet;
use std::fmt::Write;

use crate::classify::ItemType;
use crate::model::{Export, Item};

/// Namespace prefixes and URIs as the Zotero RDF translator declares them.
pub const NAMESPACES: &[(&str, &str)] = &[
    ("rdf", "http://www.w3.org/1999/02/22-rdf-syntax-ns#"),
    ("bib", "http://purl.org/net/biblio#"),
    ("dc", "http://purl.org/dc/elements/1.1/"),
    ("dcterms", "http://purl.org/dc/terms/"),
    ("prism", "http://prismstandard.org/namespaces/1.2/basic/"),
    ("foaf", "http://xmlns.com/foaf/0.1/"),
    ("vcard", "http://nwalsh.com/rdf/vCard#"),
    ("vcard2", "http://www.w3.org/2006/vcard/ns#"),
    ("link", "http://purl.org/rss/1.0/modules/link/"),
    ("z", "http://www.zotero.org/namespaces/export#"),
];

/// Line width and indentation of the serializer.
const WIDTH: i64 = 80;
const INDENT: i64 = 4;

/// A predicate or class: `(prefix, local name)`.
type Term = (&'static str, &'static str);

const RDF_SEQ: Term = ("rdf", "Seq");
const RDF_LI: Term = ("rdf", "li");
const RDF_VALUE: Term = ("rdf", "value");
const Z_ITEM_TYPE: Term = ("z", "itemType");
const Z_TYPE: Term = ("z", "type");
const Z_PMID: Term = ("z", "PMID");
const Z_PMCID: Term = ("z", "PMCID");
const DCTERMS_IS_PART_OF: Term = ("dcterms", "isPartOf");
const DCTERMS_IS_REFERENCED_BY: Term = ("dcterms", "isReferencedBy");
const DCTERMS_ABSTRACT: Term = ("dcterms", "abstract");
const DCTERMS_URI: Term = ("dcterms", "URI");
const DC_PUBLISHER: Term = ("dc", "publisher");
const DC_RELATION: Term = ("dc", "relation");
const DC_SUBJECT: Term = ("dc", "subject");
const DC_TITLE: Term = ("dc", "title");
const DC_DATE: Term = ("dc", "date");
const DC_IDENTIFIER: Term = ("dc", "identifier");
const DC_DESCRIPTION: Term = ("dc", "description");
const PRISM_VOLUME: Term = ("prism", "volume");
const PRISM_NUMBER: Term = ("prism", "number");
const BIB_AUTHORS: Term = ("bib", "authors");
const BIB_PAGES: Term = ("bib", "pages");
const FOAF_ORGANIZATION: Term = ("foaf", "Organization");
const FOAF_NAME: Term = ("foaf", "name");
const FOAF_PERSON: Term = ("foaf", "Person");
const FOAF_SURNAME: Term = ("foaf", "surname");
const FOAF_GIVEN_NAME: Term = ("foaf", "givenName");

/// The object of a statement.
enum Object {
    Literal(String),
    Uri(String),
    Node(usize),
}

/// One subject with its statements in insertion order. `about` is `None`
/// for a blank node; `kind` is its `rdf:type`, which becomes the element.
struct Subject {
    about: Option<String>,
    kind: Option<Term>,
    props: Vec<(Term, Object)>,
}

#[derive(Default)]
struct Graph {
    subjects: Vec<Subject>,
}

impl Graph {
    fn subject(&mut self, about: Option<String>, kind: Option<Term>) -> usize {
        self.subjects.push(Subject {
            about,
            kind,
            props: Vec::new(),
        });
        self.subjects.len() - 1
    }

    fn add(&mut self, subject: usize, pred: Term, object: Object) {
        self.subjects[subject].props.push((pred, object));
    }

    fn literal(&mut self, subject: usize, pred: Term, value: &str) {
        self.add(subject, pred, Object::Literal(value.to_string()));
    }
}

/// Zotero fields an item can carry, in the order the Zotero schema lists
/// them for each type (the order the translator visits `uniqueFields`).
#[derive(Clone, Copy)]
enum Field {
    Title,
    AbstractNote,
    PublicationTitle,
    Publisher,
    Type,
    Number,
    Date,
    Volume,
    Issue,
    Pages,
    Doi,
    Isbn,
    Url,
    Pmid,
    Pmcid,
    Issn,
    Extra,
}

/// Zotero schema (version 45) field order per item type, reduced to the
/// fields an export can fill. Type-specific names are shown as their base
/// field: `bookTitle`/`proceedingsTitle`/`websiteTitle` →
/// `PublicationTitle`, `university`/`institution`/`repository` →
/// `Publisher`, `thesisType`/`reportType`/`genre`/`websiteType` → `Type`,
/// `reportNumber`/`archiveID` → `Number`.
fn fields(item_type: ItemType) -> &'static [Field] {
    use Field::{
        AbstractNote, Date, Doi, Extra, Isbn, Issn, Issue, Number, Pages, Pmcid, Pmid,
        PublicationTitle, Publisher, Title, Type, Url, Volume,
    };
    match item_type {
        ItemType::JournalArticle => &[
            Title,
            AbstractNote,
            PublicationTitle,
            Publisher,
            Date,
            Volume,
            Issue,
            Pages,
            Doi,
            Url,
            Pmid,
            Pmcid,
            Issn,
            Extra,
        ],
        ItemType::Book => &[
            Title,
            AbstractNote,
            Volume,
            Date,
            Publisher,
            Isbn,
            Doi,
            Url,
            Issn,
            Extra,
        ],
        ItemType::BookSection => &[
            Title,
            AbstractNote,
            PublicationTitle,
            Volume,
            Date,
            Publisher,
            Pages,
            Isbn,
            Doi,
            Url,
            Issn,
            Extra,
        ],
        ItemType::ConferencePaper => &[
            Title,
            AbstractNote,
            PublicationTitle,
            Publisher,
            Date,
            Volume,
            Issue,
            Pages,
            Doi,
            Isbn,
            Url,
            Issn,
            Extra,
        ],
        ItemType::Preprint => &[
            Title,
            AbstractNote,
            Type,
            Publisher,
            Number,
            Date,
            Doi,
            Url,
            Extra,
        ],
        ItemType::Report => &[
            Title,
            AbstractNote,
            Number,
            Type,
            Publisher,
            Date,
            Pages,
            Doi,
            Isbn,
            Url,
            Issn,
            Extra,
        ],
        ItemType::Thesis => &[
            Title,
            AbstractNote,
            Type,
            Publisher,
            Date,
            Doi,
            Isbn,
            Url,
            Issn,
            Extra,
        ],
        ItemType::Webpage => &[
            Title,
            AbstractNote,
            PublicationTitle,
            Type,
            Date,
            Publisher,
            Doi,
            Url,
            Extra,
        ],
    }
}

/// The `rdf:type` of an item and of its container, per `generateItem` in
/// the translator. `conferencePaper` and `preprint` have no `bib:` class.
fn rdf_types(item_type: ItemType) -> (Option<Term>, Option<Term>) {
    match item_type {
        ItemType::JournalArticle => (Some(("bib", "Article")), Some(("bib", "Journal"))),
        ItemType::Book => (Some(("bib", "Book")), None),
        ItemType::BookSection => (Some(("bib", "BookSection")), Some(("bib", "Book"))),
        ItemType::ConferencePaper => (None, Some(("bib", "Journal"))),
        ItemType::Thesis => (Some(("bib", "Thesis")), None),
        ItemType::Report => (Some(("bib", "Report")), None),
        ItemType::Webpage => (Some(("bib", "Document")), Some(("z", "Website"))),
        ItemType::Preprint => (None, None),
    }
}

/// Resource identifiers as `doExport` assigns them: `urn:isbn:` for an
/// unused ISBN, else the item's URL when unused, else `#item_<id>`.
pub fn resource_uris(items: &[Item]) -> Vec<String> {
    let mut used = BTreeSet::new();
    items
        .iter()
        .map(|item| {
            if let Some(isbn) = &item.isbn {
                let uri = format!("urn:isbn:{isbn}");
                if used.insert(uri.clone()) {
                    return uri;
                }
            }
            if let Some(url) = &item.url
                && used.insert(url.clone())
            {
                return url.clone();
            }
            format!("#item_{}", item.id)
        })
        .collect()
}

/// Renders `export` as Zotero RDF.
pub fn render(export: &Export) -> String {
    let uris = resource_uris(&export.items);
    let mut graph = Graph::default();
    let mut used_issn = BTreeSet::new();
    for (index, item) in export.items.iter().enumerate() {
        add_item(&mut graph, export, item, &uris, index, &mut used_issn);
    }
    serialize(&graph)
}

fn add_item(
    graph: &mut Graph,
    export: &Export,
    item: &Item,
    uris: &[String],
    index: usize,
    used_issn: &mut BTreeSet<String>,
) {
    let (kind, container_kind) = rdf_types(item.item_type);
    let subject = graph.subject(Some(uris[index].clone()), kind);
    graph.literal(subject, Z_ITEM_TYPE, item.item_type.zotero_name());
    let container = container_kind.map(|container_kind| {
        let about = item
            .issn
            .as_ref()
            .map(|issn| format!("urn:issn:{issn}"))
            .filter(|uri| used_issn.insert(uri.clone()));
        let container = graph.subject(about, Some(container_kind));
        graph.add(subject, DCTERMS_IS_PART_OF, Object::Node(container));
        container
    });
    let organization = item.publisher.as_ref().map(|_| {
        let organization = graph.subject(None, Some(FOAF_ORGANIZATION));
        graph.add(subject, DC_PUBLISHER, Object::Node(organization));
        organization
    });
    if !item.creators.is_empty() {
        let seq = graph.subject(None, Some(RDF_SEQ));
        graph.add(subject, BIB_AUTHORS, Object::Node(seq));
        for creator in &item.creators {
            let person = graph.subject(None, Some(FOAF_PERSON));
            graph.literal(person, FOAF_SURNAME, &creator.last_name);
            if let Some(first) = &creator.first_name {
                graph.literal(person, FOAF_GIVEN_NAME, first);
            }
            graph.add(seq, RDF_LI, Object::Node(person));
        }
    }
    if item.reference_index.is_some() {
        graph.add(
            subject,
            DCTERMS_IS_REFERENCED_BY,
            Object::Uri(uris[0].clone()),
        );
    }
    for related in &item.related {
        if let Some(position) = export.items.iter().position(|other| other.id == *related) {
            graph.add(subject, DC_RELATION, Object::Uri(uris[position].clone()));
        }
    }
    for tag in &item.tags {
        graph.literal(subject, DC_SUBJECT, tag);
    }
    // "containerElement ? containerElement : resource" in the translator.
    let target = container.unwrap_or(subject);
    for field in fields(item.item_type) {
        match field {
            Field::Title => {
                if let Some(title) = &item.title {
                    graph.literal(subject, DC_TITLE, title);
                }
            }
            Field::AbstractNote => {
                if let Some(text) = &item.abstract_note {
                    graph.literal(subject, DCTERMS_ABSTRACT, text);
                }
            }
            Field::PublicationTitle => {
                if let Some(title) = &item.publication_title {
                    graph.literal(target, DC_TITLE, title);
                }
            }
            Field::Publisher => {
                if let (Some(organization), Some(name)) = (organization, &item.publisher) {
                    graph.literal(organization, FOAF_NAME, name);
                }
            }
            Field::Type => {
                if let Some(kind) = &item.type_field {
                    graph.literal(subject, Z_TYPE, kind);
                }
            }
            Field::Number => {
                if let Some(number) = &item.number {
                    graph.literal(target, PRISM_NUMBER, number);
                }
            }
            Field::Date => {
                if let Some(date) = &item.date {
                    graph.literal(subject, DC_DATE, date);
                }
            }
            Field::Volume => {
                if let Some(volume) = &item.volume {
                    graph.literal(target, PRISM_VOLUME, volume);
                }
            }
            Field::Issue => {
                if let Some(issue) = &item.issue {
                    graph.literal(target, PRISM_NUMBER, issue);
                }
            }
            Field::Pages => {
                if let Some(pages) = &item.pages {
                    graph.literal(subject, BIB_PAGES, pages);
                }
            }
            Field::Doi => {
                if let Some(doi) = &item.doi {
                    graph.literal(target, DC_IDENTIFIER, &format!("DOI {doi}"));
                }
            }
            Field::Isbn => {
                if let Some(isbn) = &item.isbn {
                    graph.literal(target, DC_IDENTIFIER, &format!("ISBN {isbn}"));
                }
            }
            Field::Issn => {
                if let Some(issn) = &item.issn {
                    graph.literal(target, DC_IDENTIFIER, &format!("ISSN {issn}"));
                }
            }
            Field::Url => {
                if let Some(url) = &item.url {
                    let term = graph.subject(None, Some(DCTERMS_URI));
                    graph.literal(term, RDF_VALUE, url);
                    graph.add(subject, DC_IDENTIFIER, Object::Node(term));
                }
            }
            Field::Pmid => {
                if let Some(pmid) = &item.pmid {
                    graph.literal(subject, Z_PMID, pmid);
                }
            }
            Field::Pmcid => {
                if let Some(pmcid) = &item.pmcid {
                    graph.literal(subject, Z_PMCID, pmcid);
                }
            }
            Field::Extra => {
                if let Some(extra) = &item.extra {
                    graph.literal(subject, DC_DESCRIPTION, extra);
                }
            }
        }
    }
}

/// A fragment of the serializer's nested text tree.
enum Node {
    Text(String),
    Tree(Vec<Node>),
}

struct Serializer<'a> {
    graph: &'a Graph,
    incoming: Vec<usize>,
    /// Prefixes in first-use order; `rdf` is always first.
    prefixes: Vec<&'static str>,
}

impl Serializer<'_> {
    fn qname(&mut self, term: Term) -> String {
        if !self.prefixes.contains(&term.0) {
            self.prefixes.push(term.0);
        }
        format!("{}:{}", term.0, term.1)
    }

    /// `subjectXMLTree`: the element for one subject with its statements
    /// as children; anonymous blank nodes with one incoming arc nest.
    fn subject_tree(&mut self, index: usize) -> Node {
        let subject = &self.graph.subjects[index];
        let mut results = Vec::new();
        for (pred, object) in &subject.props {
            let t = self.qname(*pred);
            match object {
                Object::Literal(value) => {
                    results.push(Node::Text(format!("<{t}>{}</{t}>", escape(value))));
                }
                Object::Uri(uri) => {
                    results.push(Node::Text(format!(
                        "<{t} rdf:resource=\"{}\"/>",
                        escape(uri)
                    )));
                }
                Object::Node(target) => {
                    let target = *target;
                    if let Some(uri) = &self.graph.subjects[target].about {
                        results.push(Node::Text(format!(
                            "<{t} rdf:resource=\"{}\"/>",
                            escape(uri)
                        )));
                    } else if self.incoming[target] == 1 {
                        results.push(Node::Text(format!("<{t}>")));
                        results.push(self.subject_tree(target));
                        results.push(Node::Text(format!("</{t}>")));
                    } else {
                        results.push(Node::Text(format!("<{t} rdf:nodeID=\"n{target}\"/>")));
                    }
                }
            }
        }
        let tag = match subject.kind {
            Some(kind) => self.qname(kind),
            None => "rdf:Description".to_string(),
        };
        let attrs = match &subject.about {
            Some(uri) => format!(" rdf:about=\"{}\"", escape(uri)),
            None if self.incoming[index] == 1 => String::new(),
            None => format!(" rdf:nodeID=\"n{index}\""),
        };
        Node::Tree(vec![
            Node::Text(format!("<{tag}{attrs}>")),
            Node::Tree(results),
            Node::Text(format!("</{tag}>")),
        ])
    }
}

fn serialize(graph: &Graph) -> String {
    let mut incoming = vec![0_usize; graph.subjects.len()];
    for subject in &graph.subjects {
        for (_, object) in &subject.props {
            if let Object::Node(target) = object {
                incoming[*target] += 1;
            }
        }
    }
    let roots: Vec<usize> = (0..graph.subjects.len())
        .filter(|&i| graph.subjects[i].about.is_some() || incoming[i] != 1)
        .collect();
    let mut serializer = Serializer {
        graph,
        incoming,
        prefixes: vec!["rdf"],
    };
    let tree: Vec<Node> = roots
        .into_iter()
        .map(|root| serializer.subject_tree(root))
        .collect();
    let mut head = String::from("<rdf:RDF");
    for prefix in &serializer.prefixes {
        let uri = NAMESPACES
            .iter()
            .find(|(p, _)| p == prefix)
            .map_or("", |(_, uri)| uri);
        let _ = write!(head, "\n xmlns:{prefix}=\"{}\"", escape(uri));
    }
    head.push('>');
    let document = vec![
        Node::Text(head),
        Node::Tree(tree),
        Node::Text("</rdf:RDF>".to_string()),
    ];
    let mut out = String::new();
    tree_to_string(&document, -1, &mut out);
    out
}

/// `escapeForXML` of the serializer: the four characters it replaces.
pub fn escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for c in value.chars() {
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            _ => out.push(c),
        }
    }
    out
}

/// String length as JavaScript counts it (UTF-16 code units).
fn js_len(s: &str) -> i64 {
    i64::try_from(s.encode_utf16().count()).unwrap_or(i64::MAX)
}

fn tree_to_line(nodes: &[Node], out: &mut String) {
    for node in nodes {
        match node {
            Node::Text(text) => out.push_str(text),
            Node::Tree(children) => tree_to_line(children, out),
        }
    }
}

/// `XMLtreeToString`: indent by level, fold a subtree onto one line when it
/// is short, and continue a very short previous line.
fn tree_to_string(nodes: &[Node], level: i64, out: &mut String) {
    let mut last_length: i64 = 100_000;
    let indent = INDENT * level;
    let emit = |text: &str, out: &mut String, last_length: &mut i64| {
        if *last_length < indent + 4 {
            out.pop();
            out.push(' ');
            out.push_str(text);
            out.push('\n');
            *last_length += js_len(text) + 1;
        } else {
            let line_length = indent.max(0) + js_len(text);
            for _ in 0..indent.max(0) {
                out.push(' ');
            }
            out.push_str(text);
            out.push('\n');
            *last_length = line_length;
        }
    };
    for node in nodes {
        match node {
            Node::Text(text) => emit(text, out, &mut last_length),
            Node::Tree(children) => {
                let mut substr = String::new();
                tree_to_string(children, level + 1, &mut substr);
                let mut folded = None;
                if js_len(&substr) < 10 * (WIDTH - indent) && !substr.contains("\"\"\"") {
                    let mut line = String::new();
                    tree_to_line(children, &mut line);
                    if js_len(&line) < WIDTH - indent {
                        folded = Some(format!("   {line}"));
                        substr.clear();
                    }
                }
                if !substr.is_empty() {
                    last_length = 10_000;
                }
                out.push_str(&substr);
                if let Some(line) = folded {
                    emit(&line, out, &mut last_length);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn escape_covers_the_serializer_set() {
        assert_eq!(
            escape("a & b < c > \"d\" 'e'"),
            "a &amp; b &lt; c &gt; &quot;d&quot; 'e'"
        );
    }

    #[test]
    fn short_subtrees_fold_onto_one_line_with_three_spaces() {
        let tree = vec![
            Node::Text("<alpha:beta>".into()),
            Node::Tree(vec![
                Node::Text("<b>".into()),
                Node::Tree(vec![Node::Text("<c>x</c>".into())]),
                Node::Text("</b>".into()),
            ]),
            Node::Text("</alpha:beta>".into()),
        ];
        let mut out = String::new();
        tree_to_string(&tree, 0, &mut out);
        assert_eq!(out, "<alpha:beta>\n   <b><c>x</c></b>\n</alpha:beta>\n");
    }

    #[test]
    fn a_very_short_previous_line_is_continued_like_the_serializer_does() {
        // `lastLength < indent * level + 4` in `XMLtreeToString`: a line
        // shorter than four characters plus the indent is joined with the
        // next one. Real tags are longer than that, so this only shows in
        // synthetic input, but the rule is kept for fidelity.
        let tree = vec![
            Node::Text("<a>".into()),
            Node::Tree(vec![Node::Text("<b>x</b>".into())]),
            Node::Text("</a>".into()),
        ];
        let mut out = String::new();
        tree_to_string(&tree, 0, &mut out);
        assert_eq!(out, "<a>    <b>x</b>\n</a>\n");
    }

    #[test]
    fn long_subtrees_stay_nested() {
        let long = "x".repeat(90);
        let tree = vec![
            Node::Text("<a>".into()),
            Node::Tree(vec![
                Node::Text("<b>".into()),
                Node::Tree(vec![Node::Text(format!("<c>{long}</c>"))]),
                Node::Text("</b>".into()),
            ]),
            Node::Text("</a>".into()),
        ];
        let mut out = String::new();
        tree_to_string(&tree, 0, &mut out);
        assert_eq!(
            out,
            format!("<a>\n    <b>\n        <c>{long}</c>\n    </b>\n</a>\n")
        );
    }
}
