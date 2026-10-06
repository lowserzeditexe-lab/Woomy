#[path = "../../../tests/support/mod.rs"]
mod support;
use std::{fs, sync::atomic::AtomicU8};
use woomy_ai::*;
use woomy_core::{AnalysisService, ProjectService};
#[test]
fn context_is_bounded_and_mock_cache_preserves_provenance() -> Result<()> {
    let d = tempfile::tempdir().map_err(woomy_core::Error::from)?;
    for s in ["code", "content", "meta"] {
        fs::create_dir(d.path().join(s)).map_err(woomy_core::Error::from)?;
    }
    fs::write(d.path().join("code/synthetic.rpx"), support::cafe_fixture())
        .map_err(woomy_core::Error::from)?;
    let mut p = ProjectService::create(d.path())?;
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    let functions = p.list("functions", "", 10, 0)?;
    let f = functions.items[0]["id"].as_str().unwrap();
    for budget in [2048, 4096, 8192, 32_768] {
        let c = ContextBuilder::build(&p, f, budget)?;
        assert!(c.json.len() <= budget);
        assert!(!c.json.contains(&d.path().to_string_lossy().to_string()));
        assert!(serde_json::from_str::<serde_json::Value>(&c.json).is_ok());
    }
    assert!(ContextBuilder::build(&p, f, 1).is_err());
    p.config.ai.enabled = true;
    p.config.ai.provider = "mock".into();
    p.config.ai.max_requests_session = 1;
    let mut ai = AiService::default();
    let a = ai.analyze(&p, f, &MockAiProvider)?;
    assert_eq!(a["provenance"], "AI_PROPOSED");
    let b = ai.analyze(&p, f, &MockAiProvider)?;
    assert_eq!(b["cached"], true);
    assert_eq!(ai.requests, 1);
    let f2 = functions.items[1]["id"].as_str().unwrap();
    assert!(matches!(
        ai.analyze(&p, f2, &MockAiProvider),
        Err(Error::Budget)
    ));
    assert!(ReadTools::execute(
        &p,
        "run_shell",
        &ToolArgs {
            target: "x".into(),
            limit: 1,
            offset: 0
        }
    )
    .is_err());
    assert!(ReadTools::execute(
        &p,
        "inspect_function",
        &ToolArgs {
            target: f.into(),
            limit: 1,
            offset: 0
        }
    )
    .is_ok());
    let xrefs = ReadTools::execute(
        &p,
        "inspect_xrefs",
        &ToolArgs {
            target: "0x1020".into(),
            limit: 128,
            offset: 0,
        },
    )?;
    assert!(xrefs["items"]
        .as_array()
        .unwrap()
        .iter()
        .any(|x| x["kind"] == "Call" && x["target"] == 0x1020));
    assert!(ReadTools::execute(
        &p,
        "inspect_xrefs",
        &ToolArgs {
            target: "invalid".into(),
            limit: 1,
            offset: 0,
        }
    )
    .is_err());
    Ok(())
}

struct Offline;
impl AiProvider for Offline {
    fn name(&self) -> &str {
        "offline-test"
    }
    fn model(&self) -> &str {
        "mock"
    }
    fn test_connection(&self) -> Result<()> {
        Err(Error::Network)
    }
    fn analyze(&self, _: &BoundedContext, _: u32) -> Result<Finding> {
        Err(Error::Network)
    }
}
#[test]
fn failed_network_job_can_be_retried_without_changing_facts() -> Result<()> {
    let d = tempfile::tempdir().map_err(woomy_core::Error::from)?;
    for s in ["code", "content", "meta"] {
        fs::create_dir(d.path().join(s)).map_err(woomy_core::Error::from)?;
    }
    fs::write(d.path().join("code/synthetic.rpx"), support::cafe_fixture())
        .map_err(woomy_core::Error::from)?;
    let mut p = ProjectService::create(d.path())?;
    AnalysisService::run(&mut p, &AtomicU8::new(0), |_| {})?;
    let f = p.list("functions", "", 1, 0)?.items[0]["id"]
        .as_str()
        .unwrap()
        .to_owned();
    p.config.ai.enabled = true;
    let mut ai = AiService::default();
    assert!(matches!(ai.analyze(&p, &f, &Offline), Err(Error::Network)));
    assert_eq!(
        p.rows("SELECT status FROM ai_jobs", &[])?[0]["status"],
        "WAITING_AI"
    );
    assert!(p.list("findings", "", 10, 0)?.items.is_empty());
    assert_eq!(p.list("functions", "", 10, 0)?.items.len(), 2);
    assert_eq!(
        ai.analyze(&p, &f, &MockAiProvider)?["provenance"],
        "AI_PROPOSED"
    );
    Ok(())
}
#[test]
#[ignore = "manual test: accesses OpenAI /models using OPENAI_API_KEY and WOOMY_TEST_MODEL; no generation"]
fn live_openai_connection() -> Result<()> {
    let key = load_key("manual-test")?;
    let model =
        std::env::var("WOOMY_TEST_MODEL").map_err(|_| Error::Config("set WOOMY_TEST_MODEL"))?;
    OpenAiProvider::new(key, &model)?.test_connection()
}
#[cfg(windows)]
#[test]
fn windows_credential_store_roundtrip() -> Result<()> {
    let scope = format!("woomy-test-{}", std::process::id());
    save_key(&scope, "synthetic-test-secret-not-an-api-key")?;
    let entry = keyring::Entry::new("Woomy/OpenAI", &scope).map_err(|_| Error::CredentialStore)?;
    assert_eq!(
        entry.get_password().map_err(|_| Error::CredentialStore)?,
        "synthetic-test-secret-not-an-api-key"
    );
    entry
        .delete_credential()
        .map_err(|_| Error::CredentialStore)?;
    Ok(())
}
