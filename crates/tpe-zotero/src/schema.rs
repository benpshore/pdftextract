//! Parsers for the schema, key, group and tag endpoints of the Web API v3:
//!
//! * `GET /itemTypes`, `GET /itemTypeFields?itemType=…`,
//!   `GET /itemTypeCreatorTypes?itemType=…`: lists of `{<name>, "localized"}`
//!   objects ([`parse_named_list`]);
//! * `GET /items/new?itemType=…[&linkMode=…]`: the empty template of a new
//!   item, every valid field with an empty value ([`parse_template`]);
//! * `GET /keys/current` (key in the `Zotero-API-Key` header): the key's
//!   user id and permissions ([`parse_key_info`]);
//! * `GET /users/<id>/groups`: the groups a user belongs to
//!   ([`parse_groups`]);
//! * `GET <prefix>/tags`: tags with their item counts ([`parse_tags`]);
//! * `POST <prefix>/collections` bodies ([`collection_body`]).
//!
//! All functions are pure and tested on recorded JSON.

use std::collections::BTreeMap;

use serde_json::{Map, Value, json};

use crate::client::Library;
use crate::error::ZError;

/// One entry of `/itemTypes`, `/itemTypeFields` or `/itemTypeCreatorTypes`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct NamedEntry {
    /// Internal name (`journalArticle`, `publicationTitle`, `author`, ...).
    pub name: String,
    /// Localised label (`Journal Article`, ...), empty when absent.
    pub localized: String,
}

/// Parse a list of `{"itemType"|"field"|"creatorType": name, "localized": label}`.
pub fn parse_named_list(body: &str) -> Result<Vec<NamedEntry>, ZError> {
    let value: Value = serde_json::from_str(body)?;
    let array = value
        .as_array()
        .ok_or_else(|| ZError::Parse("expected a JSON array of named entries".to_string()))?;
    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        let name = ["itemType", "field", "creatorType"]
            .iter()
            .find_map(|key| entry.get(*key).and_then(Value::as_str))
            .ok_or_else(|| {
                ZError::Parse("entry without itemType, field or creatorType".to_string())
            })?;
        out.push(NamedEntry {
            name: name.to_string(),
            localized: entry
                .get("localized")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        });
    }
    Ok(out)
}

/// Parse the template of a new item (`GET /items/new?itemType=…`): a JSON
/// object whose keys are exactly the fields valid for that type.
pub fn parse_template(body: &str) -> Result<Map<String, Value>, ZError> {
    let value: Value = serde_json::from_str(body)?;
    let object = value
        .as_object()
        .ok_or_else(|| ZError::Parse("item template is not a JSON object".to_string()))?;
    if !object.get("itemType").is_some_and(Value::is_string) {
        return Err(ZError::Parse(
            "item template without an itemType".to_string(),
        ));
    }
    Ok(object.clone())
}

/// What a key may do in one library.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
#[allow(clippy::struct_excessive_bools)] // mirrors the four flags the API reports
pub struct LibraryAccess {
    /// Read items and collections.
    pub library: bool,
    /// Download attachment files.
    pub files: bool,
    /// Read and write notes.
    pub notes: bool,
    /// Create, change and delete objects.
    pub write: bool,
}

impl LibraryAccess {
    fn from_value(value: Option<&Value>) -> Self {
        let flag = |name: &str| {
            value
                .and_then(|v| v.get(name))
                .and_then(Value::as_bool)
                .unwrap_or(false)
        };
        Self {
            library: flag("library"),
            files: flag("files"),
            notes: flag("notes"),
            write: flag("write"),
        }
    }
}

/// `GET /keys/current`: the user behind a key and its permissions. The key
/// itself is not kept.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZKeyInfo {
    /// Numeric user id (the `/users/<id>` prefix).
    pub user_id: u64,
    /// Account name, when the server sends one.
    pub username: Option<String>,
    /// Display name, when the server sends one.
    pub display_name: Option<String>,
    /// Permissions on the user's own library.
    pub user: LibraryAccess,
    /// Permissions per group: the group id as text, or `all`.
    pub groups: BTreeMap<String, LibraryAccess>,
}

impl ZKeyInfo {
    /// Permissions of this key on `library` (group access falls back to the
    /// `all` entry).
    pub fn access(&self, library: Library) -> LibraryAccess {
        match library {
            Library::User(_) => self.user,
            Library::Group(id) => self
                .groups
                .get(&id.to_string())
                .or_else(|| self.groups.get("all"))
                .copied()
                .unwrap_or_default(),
        }
    }

    /// True when the key may write to `library`.
    pub fn can_write(&self, library: Library) -> bool {
        self.access(library).write
    }
}

/// Parse the body of `GET /keys/current` or `GET /keys/<key>`.
pub fn parse_key_info(body: &str) -> Result<ZKeyInfo, ZError> {
    let value: Value = serde_json::from_str(body)?;
    let object = value
        .as_object()
        .ok_or_else(|| ZError::Parse("key info is not a JSON object".to_string()))?;
    let user_id = object
        .get("userID")
        .and_then(Value::as_u64)
        .ok_or_else(|| ZError::Parse("key info without a userID".to_string()))?;
    let access = object.get("access");
    let groups = access
        .and_then(|a| a.get("groups"))
        .and_then(Value::as_object)
        .map(|map| {
            map.iter()
                .map(|(id, v)| (id.clone(), LibraryAccess::from_value(Some(v))))
                .collect()
        })
        .unwrap_or_default();
    Ok(ZKeyInfo {
        user_id,
        username: object
            .get("username")
            .and_then(Value::as_str)
            .map(str::to_string),
        display_name: object
            .get("displayName")
            .and_then(Value::as_str)
            .map(str::to_string),
        user: LibraryAccess::from_value(access.and_then(|a| a.get("user"))),
        groups,
    })
}

/// One group from `GET /users/<id>/groups`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZGroup {
    /// Numeric group id (the `/groups/<id>` prefix).
    pub id: u64,
    /// Group version.
    pub version: u64,
    /// Display name.
    pub name: String,
    /// `Private`, `PublicClosed` or `PublicOpen`.
    pub group_type: Option<String>,
    /// Who may edit the library: `members` or `admins`.
    pub library_editing: Option<String>,
    /// `meta.numItems`, when sent.
    pub num_items: Option<u64>,
}

/// Parse the body of `GET /users/<id>/groups`.
pub fn parse_groups(body: &str) -> Result<Vec<ZGroup>, ZError> {
    let value: Value = serde_json::from_str(body)?;
    let array = value
        .as_array()
        .ok_or_else(|| ZError::Parse("expected a JSON array of groups".to_string()))?;
    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        let data = entry.get("data").and_then(Value::as_object);
        let text = |name: &str| {
            data.and_then(|d| d.get(name))
                .and_then(Value::as_str)
                .map(str::to_string)
        };
        let id = entry
            .get("id")
            .and_then(Value::as_u64)
            .or_else(|| data.and_then(|d| d.get("id")).and_then(Value::as_u64))
            .ok_or_else(|| ZError::Parse("group without an id".to_string()))?;
        out.push(ZGroup {
            id,
            version: entry
                .get("version")
                .and_then(Value::as_u64)
                .or_else(|| data.and_then(|d| d.get("version")).and_then(Value::as_u64))
                .unwrap_or(0),
            name: text("name").unwrap_or_default(),
            group_type: text("type"),
            library_editing: text("libraryEditing"),
            num_items: entry
                .get("meta")
                .and_then(|m| m.get("numItems"))
                .and_then(Value::as_u64),
        });
    }
    Ok(out)
}

/// One tag from `GET <prefix>/tags`.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ZTag {
    /// Tag text.
    pub tag: String,
    /// `0` manual, `1` automatic.
    pub tag_type: u8,
    /// `meta.numItems`, when sent.
    pub num_items: Option<u64>,
}

/// Parse the body of `GET <prefix>/tags`.
pub fn parse_tags(body: &str) -> Result<Vec<ZTag>, ZError> {
    let value: Value = serde_json::from_str(body)?;
    let array = value
        .as_array()
        .ok_or_else(|| ZError::Parse("expected a JSON array of tags".to_string()))?;
    let mut out = Vec::with_capacity(array.len());
    for entry in array {
        let tag = entry
            .get("tag")
            .and_then(Value::as_str)
            .ok_or_else(|| ZError::Parse("tag without a tag property".to_string()))?;
        let meta = entry.get("meta");
        out.push(ZTag {
            tag: tag.to_string(),
            tag_type: meta
                .and_then(|m| m.get("type"))
                .and_then(Value::as_u64)
                .and_then(|t| u8::try_from(t).ok())
                .unwrap_or(0),
            num_items: meta.and_then(|m| m.get("numItems")).and_then(Value::as_u64),
        });
    }
    Ok(out)
}

/// One object of a `POST <prefix>/collections` body. `parent` is `false`
/// in the JSON for a top-level collection, as the API requires.
pub fn collection_body(name: &str, parent: Option<&str>) -> Value {
    json!({
        "name": name,
        "parentCollection": parent.map_or(Value::Bool(false), |p| json!(p)),
        "relations": {}
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    // Shapes from the "Item Type/Field Requests" section of the basics page.
    #[test]
    fn named_lists_accept_all_three_shapes() {
        let types = parse_named_list(
            r#"[{"itemType": "book", "localized": "Book"}, {"itemType": "journalArticle", "localized": "Journal Article"}]"#,
        )
        .unwrap();
        assert_eq!(types[1].name, "journalArticle");
        assert_eq!(types[1].localized, "Journal Article");
        let fields = parse_named_list(
            r#"[{"field": "title", "localized": "Title"}, {"field": "publicationTitle", "localized": "Publication"}]"#,
        )
        .unwrap();
        assert_eq!(fields[1].name, "publicationTitle");
        let creators = parse_named_list(
            r#"[{"creatorType": "author", "localized": "Author"}, {"creatorType": "editor"}]"#,
        )
        .unwrap();
        assert_eq!(creators[1].name, "editor");
        assert_eq!(creators[1].localized, "");
        assert!(matches!(
            parse_named_list(r#"[{"localized": "x"}]"#),
            Err(ZError::Parse(_))
        ));
    }

    #[test]
    fn templates_keep_every_field() {
        let template = parse_template(
            r#"{"itemType": "journalArticle", "title": "", "creators": [{"creatorType": "author", "firstName": "", "lastName": ""}],
                "abstractNote": "", "publicationTitle": "", "volume": "", "DOI": "", "extra": "",
                "tags": [], "collections": [], "relations": {}}"#,
        )
        .unwrap();
        assert!(template.contains_key("DOI"));
        assert_eq!(template["itemType"], json!("journalArticle"));
        assert!(matches!(
            parse_template(r#"{"title": ""}"#),
            Err(ZError::Parse(_))
        ));
    }

    // Shape from the "Key" section (GET /keys/<key>).
    const KEY_INFO: &str = r#"{
      "key": "P9NiFoyLeZu2bZNvvuQPDWsd",
      "userID": 475425,
      "username": "ada",
      "displayName": "Ada L.",
      "access": {
        "user": {"library": true, "files": true, "notes": true, "write": true},
        "groups": {"all": {"library": true, "write": false}, "123": {"library": true, "write": true}}
      }
    }"#;

    #[test]
    fn key_info_reports_permissions_per_library() {
        let info = parse_key_info(KEY_INFO).unwrap();
        assert_eq!(info.user_id, 475_425);
        assert_eq!(info.username.as_deref(), Some("ada"));
        assert!(info.can_write(Library::User(475_425)));
        assert!(info.can_write(Library::Group(123)));
        assert!(!info.can_write(Library::Group(999)));
        assert!(info.access(Library::Group(999)).library);
        assert!(info.access(Library::User(1)).notes);
        let read_only =
            parse_key_info(r#"{"userID": 7, "access": {"user": {"library": true}}}"#).unwrap();
        assert!(!read_only.can_write(Library::User(7)));
        assert!(!read_only.access(Library::Group(1)).library);
        assert!(matches!(
            parse_key_info(r#"{"username": "x"}"#),
            Err(ZError::Parse(_))
        ));
    }

    #[test]
    fn groups_parse_envelope_and_data() {
        let body = r#"[{
          "id": 123, "version": 4, "links": {},
          "meta": {"created": "2020-01-01T00:00:00Z", "lastModified": "2020-01-02T00:00:00Z", "numItems": 42},
          "data": {"id": 123, "version": 4, "name": "Reading Group", "owner": 1, "type": "Private",
                   "description": "", "url": "", "libraryEditing": "members", "libraryReading": "members",
                   "fileEditing": "members"}
        }]"#;
        let groups = parse_groups(body).unwrap();
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].id, 123);
        assert_eq!(groups[0].name, "Reading Group");
        assert_eq!(groups[0].library_editing.as_deref(), Some("members"));
        assert_eq!(groups[0].num_items, Some(42));
        assert!(matches!(parse_groups("{}"), Err(ZError::Parse(_))));
    }

    #[test]
    fn tags_parse_type_and_counts() {
        let body = r#"[{"tag": "to read", "links": {}, "meta": {"type": 0, "numItems": 3}},
                       {"tag": "auto", "meta": {"type": 1}}, {"tag": "bare"}]"#;
        let tags = parse_tags(body).unwrap();
        assert_eq!(tags[0].tag, "to read");
        assert_eq!(tags[0].num_items, Some(3));
        assert_eq!(tags[1].tag_type, 1);
        assert_eq!(tags[2].tag_type, 0);
        assert!(matches!(
            parse_tags(r#"[{"meta": {}}]"#),
            Err(ZError::Parse(_))
        ));
    }

    #[test]
    fn collection_bodies() {
        assert_eq!(
            collection_body("Top", None),
            json!({"name": "Top", "parentCollection": false, "relations": {}})
        );
        assert_eq!(
            collection_body("Sub", Some("COLL2345"))["parentCollection"],
            json!("COLL2345")
        );
    }
}
