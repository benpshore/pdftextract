//! Cross-referencing: pairwise evidence between identities, union-find
//! grouping into works, a canonical member per group and a confidence.

use serde::{Deserialize, Serialize};

use crate::biblio::{IdSource, Variant};
use crate::identity::Identity;

/// Thresholds of the text evidence.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
pub struct Params {
    /// Estimated Jaccard at or above which two texts are near-duplicates.
    pub near_threshold: f64,
    /// Estimated Jaccard at or above which two texts share enough to be
    /// versions of one work on their own.
    pub text_threshold: f64,
    /// Estimated Jaccard at or above which text corroborates a title match.
    pub corroboration_threshold: f64,
}

impl Default for Params {
    fn default() -> Self {
        Self {
            near_threshold: 0.85,
            text_threshold: 0.50,
            corroboration_threshold: 0.20,
        }
    }
}

/// What links two inputs.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Relation {
    /// Identical bytes.
    ExactBytes,
    /// Identical normalised text, different bytes.
    SameText,
    /// Shingle similarity at or above the near threshold.
    NearDuplicateText,
    /// Shingle similarity between the text and near thresholds.
    SharedText,
    SameDoi,
    SameArxiv,
    /// Same title, same first-author surname, years within one.
    TitleAuthorYear,
    /// Same title, years within one, a surname missing on one side.
    TitleYear,
}

impl Relation {
    /// Whether the relation says the texts are the same (or nearly).
    pub fn is_duplicate_text(self) -> bool {
        matches!(
            self,
            Self::ExactBytes | Self::SameText | Self::NearDuplicateText
        )
    }

    /// Stable lower-case name.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::ExactBytes => "exact_bytes",
            Self::SameText => "same_text",
            Self::NearDuplicateText => "near_duplicate_text",
            Self::SharedText => "shared_text",
            Self::SameDoi => "same_doi",
            Self::SameArxiv => "same_arxiv",
            Self::TitleAuthorYear => "title_author_year",
            Self::TitleYear => "title_year",
        }
    }
}

/// One piece of evidence linking inputs `a` and `b` (indices).
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Evidence {
    pub a: usize,
    pub b: usize,
    pub relation: Relation,
    /// Confidence in `0..=1`.
    pub score: f64,
    pub detail: String,
}

/// A member's role in its group.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    /// The member to keep: published over preprint, then the most text.
    Canonical,
    /// Same bytes or same/near-same text as the canonical member.
    Duplicate,
    /// Another version of the work (different text).
    Version,
}

impl Role {
    fn rank(self) -> u8 {
        match self {
            Self::Canonical => 0,
            Self::Duplicate => 1,
            Self::Version => 2,
        }
    }
}

/// What a group is.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GroupKind {
    Single,
    /// Every member has the same bytes.
    ExactDuplicates,
    /// Every member's text matches another member's (same or near-same).
    NearDuplicates,
    /// At least one member is a different version of the work.
    Versions,
}

/// One input as seen from its group.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Member {
    pub index: usize,
    pub path: Option<String>,
    pub sha256: String,
    pub variant: Variant,
    pub role: Role,
    /// Best evidence score linking this member to the rest of the group
    /// (1 for the canonical member and singletons).
    pub link_score: f64,
}

/// A work: the inputs the evidence connects.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Group {
    /// 1-based, in order of first member.
    pub id: usize,
    pub kind: GroupKind,
    /// The weakest member link: the confidence that every member belongs.
    pub confidence: f64,
    /// Input index of the canonical member.
    pub canonical: usize,
    pub members: Vec<Member>,
    /// Indices into the report's evidence list.
    pub evidence: Vec<usize>,
}

fn years_compatible(a: Option<u16>, b: Option<u16>) -> bool {
    match (a, b) {
        (Some(x), Some(y)) => x.abs_diff(y) <= 1,
        _ => true,
    }
}

fn id_score(a: Option<IdSource>, b: Option<IdSource>) -> f64 {
    if a == Some(IdSource::Metadata) && b == Some(IdSource::Metadata) {
        0.95
    } else {
        0.80
    }
}

/// Evidence for one pair.
fn compare(a: &Identity, b: &Identity, params: &Params) -> Vec<Evidence> {
    let mut out = Vec::new();
    let mut push = |relation: Relation, score: f64, detail: String| {
        out.push(Evidence {
            a: a.index,
            b: b.index,
            relation,
            score,
            detail,
        });
    };
    if a.sha256 == b.sha256 {
        push(
            Relation::ExactBytes,
            1.0,
            format!("sha256 {}", &a.sha256[..a.sha256.len().min(12)]),
        );
        return out;
    }
    let similarity = a.text_similarity(b);
    if a.has_text_evidence() && a.text_sha256.is_some() && a.text_sha256 == b.text_sha256 {
        push(
            Relation::SameText,
            0.99,
            format!("{} normalised tokens identical", a.words),
        );
    } else if let Some(j) = similarity {
        if j >= params.near_threshold {
            push(
                Relation::NearDuplicateText,
                j.min(0.99),
                format!(
                    "estimated Jaccard {j:.2} of word {}-grams",
                    crate::text::SHINGLE_WORDS
                ),
            );
        } else if j >= params.text_threshold {
            push(
                Relation::SharedText,
                j,
                format!(
                    "estimated Jaccard {j:.2} of word {}-grams",
                    crate::text::SHINGLE_WORDS
                ),
            );
        }
    }
    let corroborated = similarity.is_some_and(|j| j >= params.corroboration_threshold);
    if let (Some(x), Some(y)) = (&a.key.doi, &b.key.doi)
        && x == y
    {
        push(
            Relation::SameDoi,
            id_score(a.key.doi_source, b.key.doi_source),
            format!("doi {x}"),
        );
    }
    if let (Some(x), Some(y)) = (&a.key.arxiv_id, &b.key.arxiv_id)
        && x == y
    {
        push(
            Relation::SameArxiv,
            id_score(a.key.arxiv_source, b.key.arxiv_source),
            format!("arxiv {x}"),
        );
    }
    if let (Some(ta), Some(tb)) = (&a.key.title_key, &b.key.title_key)
        && ta == tb
        && years_compatible(a.key.year, b.key.year)
    {
        let years = format!("years {:?}/{:?}", a.key.year, b.key.year);
        let both_undated = a.key.year.is_none() && b.key.year.is_none();
        match (&a.key.surname, &b.key.surname) {
            (Some(sa), Some(sb)) if sa == sb => {
                let mut score = 0.85;
                if corroborated {
                    score += 0.05;
                }
                if both_undated {
                    score -= 0.10;
                }
                push(
                    Relation::TitleAuthorYear,
                    score,
                    format!("title {ta:?}, surname {sa}, {years}"),
                );
            }
            (Some(_), Some(_)) => {}
            _ => {
                let mut score = 0.70;
                if corroborated {
                    score += 0.10;
                }
                if both_undated {
                    score -= 0.10;
                }
                push(Relation::TitleYear, score, format!("title {ta:?}, {years}"));
            }
        }
    }
    out
}

/// All pairwise evidence, in input order. Quadratic in the number of
/// inputs: fine for the thousands of files a library holds, not for
/// millions.
pub fn pairwise_evidence(ids: &[Identity], params: &Params) -> Vec<Evidence> {
    let mut all = Vec::new();
    for (i, a) in ids.iter().enumerate() {
        for b in &ids[i + 1..] {
            all.extend(compare(a, b, params));
        }
    }
    all
}

fn find_root(parent: &mut [usize], mut i: usize) -> usize {
    while parent[i] != i {
        parent[i] = parent[parent[i]];
        i = parent[i];
    }
    i
}

fn union(parent: &mut [usize], a: usize, b: usize) {
    let ra = find_root(parent, a);
    let rb = find_root(parent, b);
    if ra != rb {
        let (lo, hi) = if ra < rb { (ra, rb) } else { (rb, ra) };
        parent[hi] = lo;
    }
}

/// Group the identities by the evidence. `ids[k].index` must equal `k`.
pub fn group(ids: &[Identity], evidence: &[Evidence]) -> Vec<Group> {
    let n = ids.len();
    let mut parent: Vec<usize> = (0..n).collect();
    for e in evidence {
        if e.a < n && e.b < n {
            union(&mut parent, e.a, e.b);
        }
    }
    let mut roots: Vec<usize> = Vec::new();
    let mut members_of: Vec<Vec<usize>> = vec![Vec::new(); n];
    for i in 0..n {
        let root = find_root(&mut parent, i);
        if members_of[root].is_empty() {
            roots.push(root);
        }
        members_of[root].push(i);
    }
    roots.sort_unstable();
    roots
        .into_iter()
        .enumerate()
        .map(|(g, root)| build_group(g + 1, &members_of[root], ids, evidence))
        .collect()
}

fn build_group(id: usize, member_ids: &[usize], ids: &[Identity], evidence: &[Evidence]) -> Group {
    let evidence_ids: Vec<usize> = evidence
        .iter()
        .enumerate()
        .filter(|(_, e)| member_ids.contains(&e.a) && member_ids.contains(&e.b))
        .map(|(i, _)| i)
        .collect();
    let canonical = *member_ids
        .iter()
        .min_by(|&&x, &&y| {
            let ix = &ids[x];
            let iy = &ids[y];
            ix.variant
                .rank()
                .cmp(&iy.variant.rank())
                .then(iy.words.cmp(&ix.words))
                .then(ix.path.cmp(&iy.path))
                .then(x.cmp(&y))
        })
        .expect("a group has at least one member");
    let best_link = |m: usize| -> f64 {
        evidence_ids
            .iter()
            .map(|&i| &evidence[i])
            .filter(|e| e.a == m || e.b == m)
            .map(|e| e.score)
            .fold(0.0, f64::max)
    };
    let duplicate_of_canonical = |m: usize| -> bool {
        evidence_ids.iter().map(|&i| &evidence[i]).any(|e| {
            ((e.a == m && e.b == canonical) || (e.b == m && e.a == canonical))
                && e.relation.is_duplicate_text()
        })
    };
    let has_duplicate_link = |m: usize| -> bool {
        evidence_ids
            .iter()
            .map(|&i| &evidence[i])
            .any(|e| (e.a == m || e.b == m) && e.relation.is_duplicate_text())
    };
    let mut members: Vec<Member> = member_ids
        .iter()
        .map(|&m| {
            let role = if m == canonical {
                Role::Canonical
            } else if duplicate_of_canonical(m) {
                Role::Duplicate
            } else {
                Role::Version
            };
            Member {
                index: m,
                path: ids[m].path.clone(),
                sha256: ids[m].sha256.clone(),
                variant: ids[m].variant,
                role,
                link_score: if m == canonical { 1.0 } else { best_link(m) },
            }
        })
        .collect();
    members.sort_by(|x, y| {
        x.role
            .rank()
            .cmp(&y.role.rank())
            .then(x.index.cmp(&y.index))
    });
    let kind = if member_ids.len() == 1 {
        GroupKind::Single
    } else if member_ids
        .iter()
        .all(|&m| ids[m].sha256 == ids[canonical].sha256)
    {
        GroupKind::ExactDuplicates
    } else if member_ids.iter().all(|&m| has_duplicate_link(m)) {
        GroupKind::NearDuplicates
    } else {
        GroupKind::Versions
    };
    let confidence = members
        .iter()
        .filter(|m| m.role != Role::Canonical)
        .map(|m| m.link_score)
        .fold(1.0, f64::min);
    Group {
        id,
        kind,
        confidence,
        canonical,
        members,
        evidence: evidence_ids,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::identity::fixtures::{metadata, result};
    use crate::text::tests::prose;

    fn ident(index: usize, bytes: &[u8], text: &str, meta: tpe::schema::Metadata) -> Identity {
        let r = result(bytes, &[text], meta);
        Identity::build(
            index,
            Some(format!("/in/{index}.pdf")),
            bytes.len() as u64,
            tpe::schema::sha256_hex(bytes),
            "test".into(),
            Some(&r),
            vec![],
        )
    }

    fn edited(text: &str, every: usize) -> String {
        text.split(' ')
            .enumerate()
            .map(|(i, w)| if i % every == 1 { "altered" } else { w })
            .collect::<Vec<_>>()
            .join(" ")
    }

    #[test]
    fn exact_near_version_and_distinct_are_grouped_as_documented() {
        let params = Params::default();
        let base = prose(500, 21);
        let meta = metadata(
            "A Reproducible Result About Things",
            "Ada Lovelace",
            Some(2021),
            Some("10.1000/abc"),
        );
        let ids = vec![
            ident(0, b"%PDF original", &base, meta.clone()),
            ident(1, b"%PDF original", &base, meta.clone()),
            ident(2, b"%PDF resaved", &base, meta.clone()),
            ident(
                3,
                b"%PDF near",
                &edited(&base, 60),
                metadata("Microsoft Word - x.docx", "", None, None),
            ),
            ident(
                4,
                b"%PDF preprint",
                &edited(&base, 7),
                metadata(
                    "A Reproducible Result About Things",
                    "A. Lovelace",
                    Some(2020),
                    Some("10.48550/arXiv.2001.00001"),
                ),
            ),
            ident(
                5,
                b"%PDF other",
                &prose(500, 22),
                metadata(
                    "A Different Paper Entirely",
                    "Charles Babbage",
                    Some(2021),
                    Some("10.1000/zzz"),
                ),
            ),
        ];
        let evidence = pairwise_evidence(&ids, &params);
        let groups = group(&ids, &evidence);
        assert_eq!(groups.len(), 2, "{groups:#?}");
        let work = &groups[0];
        assert_eq!(work.kind, GroupKind::Versions);
        assert_eq!(work.canonical, 0, "{work:#?}");
        let roles: Vec<(usize, Role)> = work.members.iter().map(|m| (m.index, m.role)).collect();
        assert_eq!(
            roles,
            vec![
                (0, Role::Canonical),
                (1, Role::Duplicate),
                (2, Role::Duplicate),
                (3, Role::Duplicate),
                (4, Role::Version)
            ]
        );
        assert!(work.confidence >= 0.85, "{}", work.confidence);
        assert_eq!(work.members[4].variant, Variant::Preprint);
        let relations: Vec<Relation> = work
            .evidence
            .iter()
            .map(|&i| evidence[i].relation)
            .collect();
        assert!(relations.contains(&Relation::ExactBytes));
        assert!(relations.contains(&Relation::SameText));
        assert!(relations.contains(&Relation::NearDuplicateText));
        assert!(relations.contains(&Relation::TitleAuthorYear));
        assert!(relations.contains(&Relation::SameDoi));
        assert_eq!(groups[1].kind, GroupKind::Single);
        assert_eq!(groups[1].members[0].index, 5);
        assert!((groups[1].confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn exact_duplicates_form_an_exact_group() {
        let base = prose(100, 5);
        let ids = vec![
            ident(0, b"%PDF same", &base, metadata("", "", None, None)),
            ident(1, b"%PDF same", &base, metadata("", "", None, None)),
        ];
        let evidence = pairwise_evidence(&ids, &Params::default());
        let groups = group(&ids, &evidence);
        assert_eq!(groups.len(), 1);
        assert_eq!(groups[0].kind, GroupKind::ExactDuplicates);
        assert!((groups[0].confidence - 1.0).abs() < f64::EPSILON);
    }

    #[test]
    fn title_match_needs_compatible_years_and_surnames() {
        let a = ident(
            0,
            b"a",
            &prose(100, 1),
            metadata("The Same Title Twice Over", "Ann Turing", Some(2010), None),
        );
        let b = ident(
            1,
            b"b",
            &prose(100, 2),
            metadata("The Same Title Twice Over", "Bob Other", Some(2010), None),
        );
        let c = ident(
            2,
            b"c",
            &prose(100, 3),
            metadata("The Same Title Twice Over", "Ann Turing", Some(2015), None),
        );
        let d = ident(
            3,
            b"d",
            &prose(100, 4),
            metadata("The Same Title Twice Over", "", Some(2011), None),
        );
        let evidence = pairwise_evidence(&[a, b, c, d], &Params::default());
        let pairs: Vec<(usize, usize, Relation)> =
            evidence.iter().map(|e| (e.a, e.b, e.relation)).collect();
        assert_eq!(
            pairs,
            vec![(0, 3, Relation::TitleYear), (1, 3, Relation::TitleYear)],
            "{evidence:#?}"
        );
        assert!(evidence.iter().all(|e| (e.score - 0.70).abs() < 1e-9));
    }

    #[test]
    fn short_texts_give_no_text_evidence() {
        let a = ident(0, b"a", "one two three", metadata("", "", None, None));
        let b = ident(1, b"b", "one two three", metadata("", "", None, None));
        assert!(pairwise_evidence(&[a, b], &Params::default()).is_empty());
    }

    #[test]
    fn same_text_in_different_bytes_links_without_metadata() {
        let base = prose(200, 9);
        let a = ident(0, b"a", &base, metadata("", "", None, None));
        let b = ident(1, b"b", &base, metadata("", "", None, None));
        let evidence = pairwise_evidence(&[a, b], &Params::default());
        assert_eq!(evidence.len(), 1);
        assert_eq!(evidence[0].relation, Relation::SameText);
    }
}
