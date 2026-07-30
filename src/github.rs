use std::{collections::HashMap, env};

use anyhow::{Context, Result, bail};
use serde::Deserialize;

/// A GitHub REST API client that resolves the merged pull requests touching a
/// changeset file.
pub(crate) struct GithubClient {
    agent: ureq::Agent,
    base_url: String,
    token: Option<String>,
    repository: String,
    merged_prs_by_sha: HashMap<String, Vec<u64>>,
}

#[derive(Deserialize)]
struct Commit {
    sha: String,
}

#[derive(Deserialize)]
struct Pull {
    number: u64,
    merged_at: Option<String>,
}

impl GithubClient {
    /// Creates a client for `repository` (`owner/repo`). The base URL comes
    /// from the `GITHUB_API_URL` environment variable when set, and the token
    /// from `GITHUB_TOKEN`; without a token, a note about anonymous access is
    /// printed to stderr.
    pub(crate) fn new(repository: &str) -> Self {
        let base_url = env::var("GITHUB_API_URL")
            .map(|url| url.trim_end_matches('/').to_owned())
            .unwrap_or_else(|_| "https://api.github.com".to_owned());
        let token = env::var("GITHUB_TOKEN").ok();
        if token.is_none() {
            eprintln!("note: GITHUB_TOKEN is not set; accessing the GitHub API anonymously");
        }
        Self {
            agent: ureq::Agent::new_with_defaults(),
            base_url,
            token,
            repository: repository.to_owned(),
            merged_prs_by_sha: HashMap::new(),
        }
    }

    /// Returns the numbers of the merged pull requests whose commits touched
    /// `.changeset/<file_name>` on the default branch, deduplicated and sorted
    /// ascending. `Ok(None)` means the API knows no commits for the file (not
    /// pushed yet); any API error is fatal.
    pub(crate) fn merged_prs_for_changeset(&mut self, file_name: &str) -> Result<Option<Vec<u64>>> {
        let commits: Vec<Commit> = self.get_all(format!(
            "{}/repos/{}/commits?path=.changeset/{}&per_page=100",
            self.base_url, self.repository, file_name
        ))?;
        if commits.is_empty() {
            return Ok(None);
        }
        let mut numbers = Vec::new();
        for commit in commits {
            if !self.merged_prs_by_sha.contains_key(&commit.sha) {
                let pulls: Vec<Pull> = self.get_all(format!(
                    "{}/repos/{}/commits/{}/pulls?per_page=100",
                    self.base_url, self.repository, commit.sha
                ))?;
                let merged = pulls
                    .into_iter()
                    .filter(|pull| pull.merged_at.is_some())
                    .map(|pull| pull.number)
                    .collect();
                self.merged_prs_by_sha.insert(commit.sha.clone(), merged);
            }
            numbers.extend(&self.merged_prs_by_sha[&commit.sha]);
        }
        numbers.sort_unstable();
        numbers.dedup();
        Ok(Some(numbers))
    }

    // Performs a GET request against a list endpoint and collects the items
    // of all pages into one Vec, following the `Link` header's `rel="next"`
    // URL until there is none.
    fn get_all<T: serde::de::DeserializeOwned>(&self, first_url: String) -> Result<Vec<T>> {
        let mut items = Vec::new();
        let mut url = Some(first_url);
        while let Some(current) = url {
            let mut request = self
                .agent
                .get(&current)
                .header("Accept", "application/vnd.github+json")
                .header("X-GitHub-Api-Version", "2022-11-28")
                .header(
                    "User-Agent",
                    concat!("changesette/", env!("CARGO_PKG_VERSION")),
                );
            if let Some(token) = &self.token {
                request = request.header("Authorization", format!("Bearer {token}"));
            }
            let mut response = match request.call() {
                Ok(response) => response,
                Err(ureq::Error::StatusCode(code @ (401 | 403))) => bail!(
                    "GitHub API returned {code} for {current}. Anonymous rate limits are \
                     effectively always exhausted on GitHub-hosted runners; pass a token via the \
                     GITHUB_TOKEN environment variable"
                ),
                Err(ureq::Error::StatusCode(code)) => {
                    bail!("GitHub API returned {code} for {current}")
                }
                Err(err) => return Err(err).with_context(|| format!("GET {current}")),
            };
            url = next_link(&response);
            let mut page: Vec<T> = response
                .body_mut()
                .read_json()
                .with_context(|| format!("GET {current}"))?;
            items.append(&mut page);
        }
        Ok(items)
    }
}

// Extracts the next page's URL from the response's `Link` header, e.g.
// `Link: <https://api.github.com/...&page=2>; rel="next", <...>; rel="last"`
// yields the first URL. Returns None on the last page (no header or no
// `rel="next"` part).
fn next_link(response: &ureq::http::Response<ureq::Body>) -> Option<String> {
    let value = response.headers().get("link")?.to_str().ok()?;
    value.split(',').find_map(|part| {
        let (target, params) = part.split_once(';')?;
        params
            .split(';')
            .any(|param| param.trim() == "rel=\"next\"")
            .then(|| {
                target
                    .trim()
                    .strip_prefix('<')?
                    .strip_suffix('>')
                    .map(str::to_owned)
            })
            .flatten()
    })
}
