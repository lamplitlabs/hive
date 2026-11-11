import { test } from "node:test";
import assert from "node:assert/strict";
import { gitAppView, repoCount } from "./git-app-state.ts";

test("no payload or an unavailable store shows nothing", () => {
  assert.equal(gitAppView(undefined), null);
  assert.equal(gitAppView({ available: false, configured: false }), null);
});

test("an unconfigured App names the settings still empty", () => {
  const v = gitAppView({ available: true, configured: false, app_id: "4242", installation_id: "", private_key_ref: "" });
  assert.equal(v.tone, "info");
  assert.match(v.headline, /own logins/);
  assert.deepEqual(v.missing, ["git.github_app.installation_id", "git.github_app.private_key_ref"]);
  assert.deepEqual(v.repositories, []);
});

test("a configured App GitHub cannot answer for warns and keeps the fallback in words", () => {
  const v = gitAppView({ available: true, configured: true, app_id: "1", installation_id: "2", private_key_ref: "file:///k.pem", error: "git.github_app.private_key_ref: no such file or directory" });
  assert.equal(v.tone, "warn");
  assert.match(v.detail, /no such file/);
  assert.match(v.detail, /own logins/);
  assert.deepEqual(v.missing, []);
});

test("an installed App names its account and the repositories it covers", () => {
  const v = gitAppView({
    available: true,
    configured: true,
    app_id: "1",
    installation_id: "2",
    private_key_ref: "op://Private/hive-app/key",
    app: { slug: "hive-for-amistio", name: "Hive for Amistio", html_url: "" },
    install_url: "https://github.com/apps/hive-for-amistio/installations/new",
    account: { login: "amistio", type: "Organization" },
    repository_selection: "selected",
    repositories: ["amistio/hive", "amistio/site"],
    error: "",
  });
  assert.equal(v.tone, "ok");
  assert.equal(v.headline, "Hive for Amistio is installed on amistio");
  assert.match(v.detail, /covers 2 repositories of amistio/);
  assert.equal(v.installUrl, "https://github.com/apps/hive-for-amistio/installations/new");
  const all = gitAppView({ available: true, configured: true, app_id: "1", installation_id: "2", private_key_ref: "x", repository_selection: "all", account: { login: "me", type: "User" }, repositories: [] });
  assert.match(all.detail, /every repository of me/);
});

test("repoCount says one or many", () => {
  assert.equal(repoCount(1), "1 repository");
  assert.equal(repoCount(3), "3 repositories");
});
