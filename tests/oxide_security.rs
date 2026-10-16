#![cfg(feature = "pdf-oxide")]

use lopdf::content::{Content, Operation};
use lopdf::{Document, Object, ObjectId, Stream, dictionary};
use serde_json::{Value, json};
use tpe::backend::{BackendError, EncryptionProblem};
use tpe::oxide::{Context, Input, Limits, OperationOutput, execute};

const SECRET: &str = "SYNTHETIC-TARGET";
const CONTROL: &str = "PUBLIC-CONTROL";
const USER_PASSWORD: &str = "synthetic-user-only";
const OWNER_PASSWORD: &str = "synthetic-owner-only";

fn document() -> Document {
    let mut document = Document::with_version("1.7");
    let tree = document.new_object_id();
    let font = document.add_object(dictionary! {
        "Type"=>"Font", "Subtype"=>"Type1", "BaseFont"=>"Helvetica", "Encoding"=>"WinAnsiEncoding",
    });
    let mut operations = Vec::new();
    for (text, y) in [(SECRET, 700), (CONTROL, 650)] {
        operations.extend([
            Operation::new("BT", vec![]),
            Operation::new("Tf", vec!["F1".into(), 12.into()]),
            Operation::new("Td", vec![72.into(), y.into()]),
            Operation::new("Tj", vec![Object::string_literal(text)]),
            Operation::new("ET", vec![]),
        ]);
    }
    let stream = document.add_object(Stream::new(
        dictionary! {},
        Content { operations }.encode().unwrap(),
    ));
    let page = document.add_object(dictionary! {
        "Type"=>"Page", "Parent"=>tree, "Contents"=>stream,
        "MediaBox"=>vec![0.into(),0.into(),612.into(),792.into()],
        "Resources"=>dictionary!{"Font"=>dictionary!{"F1"=>font}},
    });
    document.objects.insert(
        tree,
        dictionary! {"Type"=>"Pages","Count"=>1,"Kids"=>vec![page.into()]}.into(),
    );
    let catalog = document.add_object(dictionary! {"Type"=>"Catalog","Pages"=>tree});
    document.trailer.set("Root", catalog);
    document
}

fn bytes(mut document: Document) -> Vec<u8> {
    let mut bytes = Vec::new();
    document.save_to(&mut bytes).unwrap();
    bytes
}

// The fixture helper accepts temporary JSON alongside its owned source bytes.
#[allow(clippy::needless_pass_by_value)]
fn run(
    operation: &str,
    bytes: Vec<u8>,
    password: Option<&str>,
    params: Value,
) -> Result<OperationOutput, BackendError> {
    let inputs = [Input::new(bytes)];
    let original = inputs[0].bytes.clone();
    let limits = Limits::default();
    let context = Context::new(&inputs, password, None, &limits);
    let result = execute(operation, &context, &params);
    assert_eq!(
        inputs[0].bytes, original,
        "operation mutated immutable source"
    );
    result
}

fn encrypted() -> Vec<u8> {
    run(
        "security.encrypt",
        bytes(document()),
        None,
        json!({"user_password":USER_PASSWORD,"owner_password":OWNER_PASSWORD}),
    )
    .unwrap()
    .artifacts
    .remove(0)
    .bytes
}

fn root(document: &Document) -> ObjectId {
    document
        .trailer
        .get(b"Root")
        .unwrap()
        .as_reference()
        .unwrap()
}

fn page(document: &Document) -> ObjectId {
    *document.get_pages().get(&1).unwrap()
}

fn recoverable_bytes(document: &Document) -> Vec<u8> {
    fn append(object: &Object, out: &mut Vec<u8>) {
        match object {
            Object::String(value, _) => out.extend(value),
            Object::Array(array) => array.iter().for_each(|value| append(value, out)),
            Object::Dictionary(dict) => dict.iter().for_each(|(_, value)| append(value, out)),
            Object::Stream(stream) => {
                out.extend(
                    stream
                        .decompressed_content()
                        .unwrap_or_else(|_| stream.content.clone()),
                );
                stream.dict.iter().for_each(|(_, value)| append(value, out));
            }
            _ => {}
        }
    }
    let mut out = Vec::new();
    document
        .objects
        .values()
        .for_each(|object| append(object, &mut out));
    out
}

fn contains(bytes: &[u8], text: &str) -> bool {
    bytes
        .windows(text.len())
        .any(|window| window == text.as_bytes())
}

#[test]
fn aes256_roundtrip_authentication_and_independent_decryption() {
    let original = bytes(document());
    let output = run(
        "security.encrypt",
        original.clone(),
        None,
        json!({"user_password":USER_PASSWORD,"owner_password":OWNER_PASSWORD}),
    )
    .unwrap();
    let report = serde_json::to_string(&output.data).unwrap();
    assert!(!report.contains(USER_PASSWORD));
    assert!(!report.contains(OWNER_PASSWORD));
    assert_eq!(output.data["source_preserved"], true);
    assert_eq!(
        output.data["compatibility_repair"],
        "mandatory_revision_6_Perms_added"
    );
    assert_eq!(output.artifacts.len(), 1);
    let encrypted = &output.artifacts[0].bytes;
    let mut independent = Document::load_mem(encrypted).unwrap();
    assert!(independent.is_encrypted());
    let encryption = independent
        .trailer
        .get(b"Encrypt")
        .unwrap()
        .as_reference()
        .unwrap();
    assert_eq!(
        independent
            .get_dictionary(encryption)
            .unwrap()
            .get(b"Perms")
            .unwrap()
            .as_str()
            .unwrap()
            .len(),
        16
    );
    assert!(independent.decrypt("wrong-synthetic-password").is_err());
    independent.decrypt(USER_PASSWORD).unwrap();
    // lopdf's unauthenticated reader intentionally loads only Encrypt.
    // A password-aware reload is required to independently inspect the body.
    let independent = Document::load_mem_with_options(
        encrypted,
        lopdf::LoadOptions::with_password(USER_PASSWORD),
    )
    .unwrap();
    assert_eq!(independent.get_pages().len(), 1);
    let text = independent.extract_text(&[1]).unwrap();
    assert!(text.contains(SECRET));
    assert!(text.contains(CONTROL));
    for password in [USER_PASSWORD, OWNER_PASSWORD] {
        let independent =
            Document::load_mem_with_options(encrypted, lopdf::LoadOptions::with_password(password))
                .unwrap();
        assert!(independent.extract_text(&[1]).unwrap().contains(SECRET));
        let output = run(
            "security.decrypt",
            encrypted.clone(),
            Some(password),
            json!({}),
        )
        .unwrap();
        let result = Document::load_mem(&output.artifacts[0].bytes).unwrap();
        assert!(!result.is_encrypted());
        let text = result.extract_text(&[1]).unwrap();
        assert!(text.contains(SECRET));
        assert!(text.contains(CONTROL));
    }
    assert_ne!(original, *encrypted);
}

#[test]
fn aes256_empty_user_and_normalized_unicode_passwords_keep_all_pages() {
    for (user, owner) in [
        ("", "synthetic-owner"),
        ("synthetic-\u{00ad}user", "synthetic-\u{2168}-owner"),
    ] {
        let original = bytes(document());
        let output = run(
            "security.encrypt",
            original.clone(),
            None,
            json!({"user_password":user,"owner_password":owner}),
        )
        .unwrap();
        for password in [user, owner] {
            // lopdf's reader authenticates a normalized password but passes
            // raw bytes to key derivation; supply the required SASLprep bytes.
            // The public operation below must still accept the original input.
            let normalized = stringprep::saslprep(password).unwrap();
            let independent = Document::load_mem_with_options(
                &output.artifacts[0].bytes,
                lopdf::LoadOptions::with_password(&normalized),
            )
            .unwrap();
            assert_eq!(independent.get_pages().len(), 1);
            let text = independent.extract_text(&[1]).unwrap();
            assert!(text.contains(SECRET));
            assert!(text.contains(CONTROL));
            let decrypted = run(
                "security.decrypt",
                output.artifacts[0].bytes.clone(),
                Some(password),
                json!({}),
            )
            .unwrap();
            let decoded = Document::load_mem(&decrypted.artifacts[0].bytes).unwrap();
            let text = decoded.extract_text(&[1]).unwrap();
            assert!(text.contains(SECRET));
            assert!(text.contains(CONTROL));
        }
        assert_ne!(original, output.artifacts[0].bytes);
    }
}

#[test]
fn aes256_rejects_prohibited_or_expanding_passwords_before_writing() {
    for (user, owner) in [
        ("synthetic\u{0007}".to_owned(), OWNER_PASSWORD.to_owned()),
        (USER_PASSWORD.to_owned(), "synthetic\u{0007}".to_owned()),
        (USER_PASSWORD.to_owned(), "\u{00ad}".to_owned()),
        // U+337F SQUARE CORPORATION normalizes from 3 bytes to 12 bytes.
        ("\u{337f}".repeat(11), OWNER_PASSWORD.to_owned()),
        (USER_PASSWORD.to_owned(), "\u{337f}".repeat(11)),
    ] {
        let result = run(
            "security.encrypt",
            bytes(document()),
            None,
            json!({"user_password":user,"owner_password":owner}),
        );
        assert!(matches!(result, Err(BackendError::Malformed(_))));
    }
}

#[test]
fn encrypted_missing_wrong_and_successful_passwords_are_distinct() {
    let encrypted = encrypted();
    assert!(matches!(
        run("security.decrypt", encrypted.clone(), None, json!({})),
        Err(BackendError::Encrypted(EncryptionProblem::PasswordRequired))
    ));
    assert!(matches!(
        run(
            "security.decrypt",
            encrypted.clone(),
            Some("wrong"),
            json!({})
        ),
        Err(BackendError::Encrypted(EncryptionProblem::WrongPassword))
    ));
    let inspection = run("security.inspect", encrypted.clone(), None, json!({})).unwrap();
    assert_eq!(inspection.data["encrypted"], true);
    assert!(inspection.data["page_count"].is_null());
    let wrong = run(
        "security.inspect",
        encrypted.clone(),
        Some("wrong"),
        json!({}),
    )
    .unwrap();
    assert_eq!(wrong.data["authentication"], "wrong_password");
    let correct = run(
        "security.inspect",
        encrypted,
        Some(USER_PASSWORD),
        json!({}),
    )
    .unwrap();
    assert_eq!(correct.data["authentication"], "authenticated");
    assert_eq!(correct.data["page_count"], 1);
    assert_eq!(correct.data["encryption_dictionary"]["revision"], 6);
}

#[test]
fn encryption_rejects_legacy_and_silent_password_truncation() {
    for algorithm in ["rc4", "aes128"] {
        assert!(matches!(
            run(
                "security.encrypt",
                bytes(document()),
                None,
                json!({"user_password":"u","owner_password":"o","algorithm":algorithm})
            ),
            Err(BackendError::Unsupported(_))
        ));
    }
    let error = run(
        "security.encrypt",
        bytes(document()),
        None,
        json!({"user_password":"secret-never-echo".repeat(10),"owner_password":"owner"}),
    )
    .unwrap_err();
    assert!(matches!(error, BackendError::Malformed(_)));
    assert!(!error.to_string().contains("secret-never-echo"));
}

#[test]
fn text_redaction_removes_target_from_decoded_objects_and_preserves_control() {
    let source = bytes(document());
    let original = Document::load_mem(&source).unwrap();
    assert!(contains(&recoverable_bytes(&original), SECRET));
    let output = run(
        "security.redact_text",
        source,
        None,
        json!({"regions":[{"page":1,"rect":[60,690,250,720]}]}),
    )
    .unwrap();
    assert!(output.data["report"]["glyphs_removed"].as_u64().unwrap() > 0);
    assert_eq!(output.data["global_secret_removal_verified"], false);
    let result = Document::load_mem(&output.artifacts[0].bytes).unwrap();
    assert!(!contains(&recoverable_bytes(&result), SECRET));
    let text = result.extract_text(&[1]).unwrap();
    assert!(!text.contains(SECRET));
    assert!(text.contains(CONTROL));
    // The original remains a useful negative control, including its target.
    assert!(original.extract_text(&[1]).unwrap().contains(SECRET));
}

#[test]
fn redaction_outside_target_does_not_claim_global_secret_removal() {
    let output = run(
        "security.redact_text",
        bytes(document()),
        None,
        json!({"regions":[{"page":1,"rect":[400,400,500,500]}]}),
    )
    .unwrap();
    assert_eq!(output.data["report"]["glyphs_removed"], 0);
    let result = Document::load_mem(&output.artifacts[0].bytes).unwrap();
    assert!(result.extract_text(&[1]).unwrap().contains(SECRET));
    assert_eq!(output.data["global_secret_removal_verified"], false);
}

#[test]
fn redaction_rejects_pages_regions_and_unsupported_carriers() {
    for params in [
        json!({"regions":[]}),
        json!({"regions":[{"page":0,"rect":[1,2,3,4]}]}),
        json!({"regions":[{"page":1,"rect":[4,2,3,4]}]}),
    ] {
        assert!(run("security.redact_text", bytes(document()), None, params).is_err());
    }
    for extra in [
        b"q /Im0 Do Q".as_slice(),
        b"/Span << /ActualText (ALTERNATE-SECRET) >> BDC EMC",
        b"BI /W 1 /H 1 /BPC 8 /CS /G ID \0 EI",
    ] {
        let mut document = document();
        let page = page(&document);
        let original = document
            .get_object(page)
            .unwrap()
            .as_dict()
            .unwrap()
            .get(b"Contents")
            .unwrap()
            .clone();
        let additional = document.add_object(Stream::new(dictionary! {}, extra.to_vec()));
        document
            .get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Contents", vec![original, additional.into()]);
        assert!(matches!(
            run(
                "security.redact_text",
                bytes(document),
                None,
                json!({"regions":[{"page":1,"rect":[60,690,250,720]}]})
            ),
            Err(BackendError::Unsupported(_))
        ));
    }
}

#[test]
fn redaction_rejects_damaged_array_stream_instead_of_silently_skipping_it() {
    let mut document = document();
    let page = page(&document);
    let original = document
        .get_object(page)
        .unwrap()
        .as_dict()
        .unwrap()
        .get(b"Contents")
        .unwrap()
        .clone();
    let damaged = document.add_object(Stream::new(
        dictionary! {"Filter"=>"ASCIIHexDecode"},
        b"GG>".to_vec(),
    ));
    document
        .get_object_mut(page)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Contents", vec![original, damaged.into()]);
    assert!(
        run(
            "security.redact_text",
            bytes(document),
            None,
            json!({"regions":[{"page":1,"rect":[60,690,250,720]}]})
        )
        .is_err()
    );
}

fn catalog_secrets(shared_attachment: bool) -> Document {
    let mut document = document();
    let info =
        document.add_object(dictionary! {"Title"=>Object::string_literal("SYNTHETIC-INFO-SECRET")});
    document.trailer.set("Info", info);
    let metadata = document.add_object(Stream::new(
        dictionary! {"Type"=>"Metadata","Subtype"=>"XML"},
        b"<test>SYNTHETIC-XMP-SECRET</test>".to_vec(),
    ));
    let action = document.add_object(
        dictionary! {"S"=>"JavaScript","JS"=>Object::string_literal("/* SYNTHETIC-JS-SECRET */")},
    );
    let embedded = document.add_object(Stream::new(
        dictionary! {"Type"=>"EmbeddedFile"},
        b"SYNTHETIC-ATTACHMENT-SECRET".to_vec(),
    ));
    let file = document.add_object(dictionary!{"Type"=>"Filespec","F"=>Object::string_literal("synthetic.txt"),"EF"=>dictionary!{"F"=>embedded}});
    let names = document.add_object(dictionary!{"EmbeddedFiles"=>dictionary!{"Names"=>vec![Object::string_literal("synthetic.txt"),file.into()]} });
    let root = root(&document);
    let catalog = document
        .get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap();
    catalog.set("Metadata", metadata);
    catalog.set("OpenAction", action);
    catalog.set("Names", names);
    if shared_attachment {
        let annotation = document.add_object(dictionary!{"Type"=>"Annot","Subtype"=>"FileAttachment","FS"=>file,"Rect"=>vec![0.into(),0.into(),10.into(),10.into()]});
        let page = page(&document);
        document
            .get_object_mut(page)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Annots", vec![annotation.into()]);
    }
    document
}

#[test]
fn catalog_sanitization_removes_named_carriers_and_keeps_page_text() {
    let source = bytes(catalog_secrets(false));
    let output = run("security.sanitize", source.clone(), None, json!({})).unwrap();
    assert_eq!(output.data["comprehensive_sanitization"], false);
    let result = Document::load_mem(&output.artifacts[0].bytes).unwrap();
    let original = Document::load_mem(&source).unwrap();
    let recovered = recoverable_bytes(&result);
    for secret in [
        "SYNTHETIC-INFO-SECRET",
        "SYNTHETIC-XMP-SECRET",
        "SYNTHETIC-JS-SECRET",
        "SYNTHETIC-ATTACHMENT-SECRET",
    ] {
        assert!(contains(&recoverable_bytes(&original), secret));
        assert!(
            !contains(&recovered, secret),
            "catalog secret remains: {secret}"
        );
    }
    assert!(result.extract_text(&[1]).unwrap().contains(CONTROL));
    assert!(result.extract_text(&[1]).unwrap().contains(SECRET));
}

#[test]
fn shared_attachment_survives_catalog_scope_and_is_explicitly_out_of_scope() {
    let output = run(
        "security.sanitize",
        bytes(catalog_secrets(true)),
        None,
        json!({}),
    )
    .unwrap();
    let result = Document::load_mem(&output.artifacts[0].bytes).unwrap();
    assert!(contains(
        &recoverable_bytes(&result),
        "SYNTHETIC-ATTACHMENT-SECRET"
    ));
    assert_eq!(output.data["scope"], "document_catalog_and_info");
    assert!(
        output
            .warnings
            .iter()
            .any(|warning| warning.contains("attachment annotations"))
    );
}

#[test]
fn compliance_inspection_reports_failures_without_certification_claims() {
    for standard in [
        "PDF/A-1a",
        "PDF/A-1b",
        "PDF/A-2a",
        "PDF/A-2b",
        "PDF/A-2u",
        "PDF/A-3a",
        "PDF/A-3b",
        "PDF/A-3u",
        "PDF/UA-1",
        "PDF/UA-2",
        "PDF/X-1a:2001",
        "PDF/X-1a:2003",
        "PDF/X-3:2002",
        "PDF/X-3:2003",
        "PDF/X-4",
        "PDF/X-4p",
        "PDF/X-5g",
        "PDF/X-5n",
        "PDF/X-5pg",
        "PDF/X-6",
    ] {
        let result = run(
            "compliance.validate",
            bytes(document()),
            None,
            json!({"standard":standard}),
        )
        .unwrap();
        assert_eq!(result.data["independent_certification"], false);
        assert_eq!(
            result.data["upstream_is_compliant"], false,
            "unprepared fixture passed {standard}"
        );
        assert!(!result.data["errors"].as_array().unwrap().is_empty());
        assert!(result.artifacts.is_empty());
    }
}

#[test]
fn pdfa_candidate_retains_fresh_failures_and_source_negative_control() {
    let source = bytes(document());
    let output = run(
        "compliance.convert_pdf_a",
        source.clone(),
        None,
        json!({"standard":"PDF/A-2b"}),
    )
    .unwrap();
    assert_eq!(output.data["independent_certification"], false);
    assert_eq!(output.data["validation"]["upstream_is_compliant"], false);
    assert!(
        output.data["validation"]["errors"]
            .as_array()
            .unwrap()
            .iter()
            .any(|error| error["code"] == "FontNotEmbedded")
    );
    let candidate = &output.artifacts[0].bytes;
    assert_ne!(candidate, &source);
    assert!(
        Document::load_mem(candidate)
            .unwrap()
            .extract_text(&[1])
            .unwrap()
            .contains(CONTROL)
    );
    let fresh = run(
        "compliance.validate",
        candidate.clone(),
        None,
        json!({"standard":"PDF/A-2b"}),
    )
    .unwrap();
    assert_eq!(fresh.data["upstream_is_compliant"], false);
}

fn signed_placeholder(range: Vec<Object>, cycle: bool, inherited: bool) -> Document {
    let mut document = document();
    let value = document.add_object(dictionary!{"Type"=>"Sig","SubFilter"=>"adbe.pkcs7.detached","Name"=>Object::string_literal("Synthetic placeholder, not a signature"),"ByteRange"=>range,"Contents"=>Object::string_literal(vec![0_u8;16])});
    let leaf = document.add_object(dictionary! {"FT"=>"Sig","V"=>value});
    let top = if inherited {
        document
            .get_object_mut(leaf)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .remove(b"FT");
        document.add_object(dictionary! {"FT"=>"Sig","Kids"=>vec![leaf.into()]})
    } else {
        leaf
    };
    if cycle {
        document
            .get_object_mut(leaf)
            .unwrap()
            .as_dict_mut()
            .unwrap()
            .set("Kids", vec![top.into()]);
    }
    let form = document.add_object(dictionary! {"Fields"=>vec![top.into()]});
    let root = root(&document);
    document
        .get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("AcroForm", form);
    document
}

#[test]
fn signature_inspection_handles_unsigned_inherited_fields_and_partial_coverage() {
    let empty = run("signatures.inspect", bytes(document()), None, json!({})).unwrap();
    assert!(empty.data["signatures"].as_array().unwrap().is_empty());
    let output = run(
        "signatures.inspect",
        bytes(signed_placeholder(
            vec![0.into(), 20.into(), 30.into(), 20.into()],
            false,
            true,
        )),
        None,
        json!({}),
    )
    .unwrap();
    let signatures = output.data["signatures"].as_array().unwrap();
    assert_eq!(signatures.len(), 1);
    assert_eq!(signatures[0]["byte_range_valid"], true);
    assert_eq!(signatures[0]["claimed_ranges_end_at_eof"], false);
    assert_eq!(signatures[0]["whole_document_coverage_verified"], false);
    assert_eq!(signatures[0]["cryptographic_status"], "not_checked");
    assert_eq!(output.data["trust_verified"], false);
}

#[test]
fn signature_inspection_rejects_cycles_and_does_not_filter_bad_byte_range_values() {
    let cycle = run(
        "signatures.inspect",
        bytes(signed_placeholder(vec![], true, false)),
        None,
        json!({}),
    );
    assert!(matches!(cycle, Err(BackendError::Malformed(_))));
    for range in [
        vec![0.into(), Object::string_literal("20"), 30.into(), 20.into()],
        vec![0.into(), 20.into(), 10.into(), 20.into()],
        vec![0.into(), 20.into(), 30.into(), i64::MAX.into()],
    ] {
        let output = run(
            "signatures.inspect",
            bytes(signed_placeholder(range, false, false)),
            None,
            json!({}),
        )
        .unwrap();
        assert_eq!(output.data["signatures"][0]["byte_range_valid"], false);
        assert_eq!(
            output.data["signatures"][0]["claimed_ranges_end_at_eof"],
            false
        );
        assert_eq!(
            output.data["signatures"][0]["whole_document_coverage_verified"],
            false
        );
    }
}

#[test]
fn rewrites_reject_filled_signature_values_but_allow_blank_fields() {
    let signed = bytes(signed_placeholder(
        vec![0.into(), 20.into(), 30.into(), 20.into()],
        false,
        false,
    ));
    for (operation, params) in [
        (
            "security.encrypt",
            json!({"user_password":"u","owner_password":"o"}),
        ),
        ("security.decrypt", json!({})),
        ("security.sanitize", json!({})),
        (
            "security.redact_text",
            json!({"regions":[{"page":1,"rect":[60,690,250,720]}]}),
        ),
        ("compliance.convert_pdf_a", json!({"standard":"PDF/A-2b"})),
    ] {
        let error = run(operation, signed.clone(), None, params).unwrap_err();
        assert!(matches!(error, BackendError::Unsupported(_)));
        assert!(error.to_string().contains("signature"));
    }
    for null_value in [false, true] {
        let mut blank = signed_placeholder(vec![], false, false);
        for object in blank.objects.values_mut() {
            if let Ok(dict) = object.as_dict_mut()
                && dict
                    .get(b"FT")
                    .ok()
                    .and_then(|object| object.as_name().ok())
                    == Some(b"Sig".as_slice())
            {
                if null_value {
                    dict.set("V", Object::Null);
                } else {
                    dict.remove(b"V");
                }
            }
        }
        let output = run("security.decrypt", bytes(blank), None, json!({})).unwrap();
        assert_eq!(output.artifacts.len(), 1);
    }
}

#[test]
fn catalog_certification_reference_blocks_rewrite_without_an_acroform() {
    let mut document = document();
    let signature = document.add_object(dictionary! {
        "Type" => "Sig", "ByteRange" => vec![0.into(), 10.into(), 20.into(), 10.into()],
    });
    let root = root(&document);
    document
        .get_object_mut(root)
        .unwrap()
        .as_dict_mut()
        .unwrap()
        .set("Perms", dictionary! {"DocMDP" => signature});
    let input = bytes(document);
    let inspection = run("signatures.inspect", input.clone(), None, json!({})).unwrap();
    assert_eq!(
        inspection.data["catalog_signature_permissions_present"],
        true
    );
    assert!(matches!(
        run("security.sanitize", input, None, json!({})),
        Err(BackendError::Unsupported(_))
    ));
}

#[cfg(feature = "pdf-oxide-signatures")]
#[test]
fn offline_verification_rejects_invalid_cms_without_trust_or_network_claim() {
    let output = run(
        "signatures.verify",
        bytes(signed_placeholder(
            vec![0.into(), 20.into(), 30.into(), 20.into()],
            false,
            false,
        )),
        None,
        json!({}),
    )
    .unwrap();
    let status = output.data["signatures"][0]["cryptographic_status"]
        .as_str()
        .unwrap();
    assert!(matches!(
        status,
        "invalid" | "invalid_cms" | "unsupported_or_incomplete"
    ));
    assert_eq!(output.data["trust_verified"], false);
    assert_eq!(output.data["network_used"], false);
}

#[cfg(not(feature = "pdf-oxide-signatures"))]
#[test]
fn offline_verification_without_feature_is_explicitly_unsupported() {
    assert!(matches!(
        run("signatures.verify", bytes(document()), None, json!({})),
        Err(BackendError::Unsupported(_))
    ));
}

#[test]
fn output_item_and_page_limits_fail_without_returning_artifacts() {
    let inputs = [Input::new(bytes(document()))];
    let output_limit = Limits {
        max_output_bytes: 128,
        ..Limits::default()
    };
    let context = Context::new(&inputs, None, None, &output_limit);
    assert!(matches!(
        execute("security.decrypt", &context, &json!({})),
        Err(BackendError::Limit(_))
    ));
    let items = Limits {
        max_items: 1,
        ..Limits::default()
    };
    let context = Context::new(&inputs, None, None, &items);
    assert!(matches!(
        execute(
            "security.redact_text",
            &context,
            &json!({"regions":[{"page":1,"rect":[60,690,250,720]},{"page":1,"rect":[60,640,250,670]}]})
        ),
        Err(BackendError::Limit(_))
    ));
    let limits = Limits::default();
    let selected = Context::new(&inputs, None, Some((1, 1)), &limits);
    assert!(matches!(
        execute("security.sanitize", &selected, &json!({})),
        Err(BackendError::Unsupported(_))
    ));
}

#[test]
fn malformed_inputs_and_unknown_parameters_do_not_produce_derivatives() {
    for operation in [
        "security.inspect",
        "security.decrypt",
        "security.sanitize",
        "signatures.inspect",
    ] {
        assert!(run(operation, b"synthetic-not-a-pdf".to_vec(), None, json!({})).is_err());
        assert!(
            run(
                operation,
                bytes(document()),
                None,
                json!({"output_path":"/tmp/not-authorized"})
            )
            .is_err()
        );
    }
    assert!(matches!(
        run(
            "compliance.validate",
            bytes(document()),
            None,
            json!({"standard":"invented-standard"})
        ),
        Err(BackendError::Unsupported(_))
    ));
}
