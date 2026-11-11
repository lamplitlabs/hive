//! `GET /api/v1/git-app`: what the Settings -> Git panel shows about the
//! GitHub App Hive reads and writes repositories as (ADR-070 step 3): the
//! three settings, the installation's account and the repositories it
//! covers as the App API answers, and the link that installs the App.
//! The brain signs a JWT with the App's key to ask; the answer never holds
//! the key or a token. Writes go through `POST /api/v1/settings`.

use crate::httpx::{Request, ResponseWriter};
use crate::server::Handler;
use serde_json::json;

impl Handler {
    pub(crate) fn addition_git_app(&self, w: &mut ResponseWriter, r: &Request, parts: Vec<String>) {
        if parts.len() != 1 {
            crate::addition_not_found(w);
            return;
        }
        if !crate::addition_method(w, r, vec!["GET".to_string()])
            || !self.addition_available(w, r, vec![])
        {
            return;
        }
        let Some(store) = self.store.as_deref() else {
            crate::write_json(w, 200, json!({"available": false, "configured": false}));
            return;
        };
        let mut v = hive_runner::gitapp::status(store);
        v["available"] = json!(true);
        crate::write_json(w, 200, v);
    }
}

#[cfg(test)]
mod tests {
    use crate::test_helpers_http as http;

    #[test]
    fn git_app_reports_an_unconfigured_app_without_asking_github() {
        let _serial = crate::test_helpers::serial();
        let s = http::additions_store(true);
        let h = http::handler(&s);
        let (status, data) = http::additions_request(h.clone(), "GET", "/api/v1/git-app", "");
        assert_eq!(status, 200, "{data:?}");
        assert_eq!(data["available"], true);
        assert_eq!(data["configured"], false);
        assert_eq!(data["install_url"], "");
        assert_eq!(data["repositories"], serde_json::json!([]));
        let (status, _) = http::additions_request(h.clone(), "POST", "/api/v1/git-app", "{}");
        assert_eq!(status, 405);
        s.set_setting("human", "git.github_app.app_id", "4242")
            .unwrap();
        s.set_setting("human", "git.github_app.installation_id", "777")
            .unwrap();
        s.set_setting(
            "human",
            "git.github_app.private_key_ref",
            "file:///nonexistent/hive-app.pem",
        )
        .unwrap();
        let (status, data) = http::additions_request(h, "GET", "/api/v1/git-app", "");
        assert_eq!(status, 200);
        assert_eq!(data["configured"], true);
        assert_eq!(data["app_id"], "4242");
        let err = data["error"].as_str().unwrap();
        assert!(err.contains("git.github_app.private_key_ref"), "{err}");
    }
}
