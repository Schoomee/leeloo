//! Liste des VM Linux Azure via Resource Graph (logique pure, sans GTK).
//!
//! Resource Graph (~1 s) plutôt que `az vm list --show-details` (~1 min :
//! un appel par VM). L'état d'alimentation vient de `extended.instanceView`.

use crate::session::AzureVmSession;
use serde::Deserialize;
use std::collections::HashMap;
use std::fmt;

const GRAPH_QUERY: &str = "Resources \
| where type =~ 'microsoft.compute/virtualmachines' \
| where properties.storageProfile.osDisk.osType =~ 'Linux' \
| project subscriptionId, resourceGroup, name, location, tags, \
power = tostring(properties.extended.instanceView.powerState.code)";

/// Plafond de lignes renvoyées par `az graph query --first`.
pub const GRAPH_PAGE_SIZE: u32 = 1000;

#[derive(Debug)]
pub struct ParseError(serde_json::Error);

impl fmt::Display for ParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "réponse Resource Graph invalide : {}", self.0)
    }
}

impl std::error::Error for ParseError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmRow {
    pub subscription_id: String,
    pub resource_group: String,
    pub name: String,
    pub location: String,
    /// `running`, `deallocated`… (préfixe `PowerState/` retiré).
    pub power: Option<String>,
    pub tags: HashMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VmList {
    pub rows: Vec<VmRow>,
    /// Vrai si Resource Graph a plus de lignes que celles reçues.
    pub truncated: bool,
}

#[derive(Deserialize)]
struct GraphOutput {
    #[serde(default)]
    data: Vec<RawRow>,
    #[serde(default)]
    total_records: Option<u64>,
}

#[derive(Deserialize)]
struct RawRow {
    #[serde(rename = "subscriptionId")]
    subscription_id: String,
    #[serde(rename = "resourceGroup")]
    resource_group: String,
    name: String,
    #[serde(default)]
    location: String,
    #[serde(default)]
    tags: Option<HashMap<String, serde_json::Value>>,
    #[serde(default)]
    power: Option<String>,
}

/// `az graph query`, limité à une subscription (`Some(id)`) ou à toutes (`None`).
pub fn graph_query_argv(subscription_id: Option<&str>) -> Vec<String> {
    let mut argv: Vec<String> = ["az", "graph", "query", "--first"]
        .map(str::to_owned)
        .to_vec();
    argv.push(GRAPH_PAGE_SIZE.to_string());
    argv.extend(["-o".to_owned(), "json".to_owned()]);
    if let Some(id) = subscription_id {
        argv.extend(["--subscriptions".to_owned(), id.to_owned()]);
    }
    argv.extend(["-q".to_owned(), GRAPH_QUERY.to_owned()]);
    argv
}

pub fn current_subscription_argv() -> Vec<String> {
    ["az", "account", "show", "--query", "id", "-o", "tsv"]
        .map(str::to_owned)
        .to_vec()
}

/// Démarre une VM arrêtée (`az vm start`).
pub fn start_vm_argv(row: &VmRow) -> Vec<String> {
    [
        "az",
        "vm",
        "start",
        "--subscription",
        &row.subscription_id,
        "--resource-group",
        &row.resource_group,
        "--name",
        &row.name,
    ]
    .map(str::to_owned)
    .to_vec()
}

/// Analyse la sortie JSON ; lignes triées par subscription, resource group
/// (sans tenir compte de la casse), puis nom.
pub fn parse_graph_output(json: &str) -> Result<VmList, ParseError> {
    let output: GraphOutput = serde_json::from_str(json).map_err(ParseError)?;
    let received = output.data.len() as u64;
    let mut rows: Vec<VmRow> = output
        .data
        .into_iter()
        .map(|raw| VmRow {
            subscription_id: raw.subscription_id,
            resource_group: raw.resource_group,
            name: raw.name,
            location: raw.location,
            power: raw
                .power
                .map(|code| code.trim_start_matches("PowerState/").to_owned())
                .filter(|code| !code.is_empty()),
            tags: raw
                .tags
                .unwrap_or_default()
                .into_iter()
                .map(|(key, value)| {
                    let text = match value {
                        serde_json::Value::String(text) => text,
                        other => other.to_string(),
                    };
                    (key, text)
                })
                .collect(),
        })
        .collect();
    rows.sort_by_key(|row| {
        (
            row.subscription_id.clone(),
            row.resource_group.to_lowercase(),
            row.name.to_lowercase(),
        )
    });
    Ok(VmList {
        rows,
        truncated: output.total_records.is_some_and(|total| total > received),
    })
}

/// Encodage d'un segment d'URL : seuls `A-Za-z0-9-._~` restent tels quels.
fn percent_encode(segment: &str) -> String {
    segment
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'.' | b'_' | b'~' => {
                char::from(byte).to_string()
            }
            other => format!("%{other:02X}"),
        })
        .collect()
}

impl VmRow {
    /// Valeur d'un tag (clé insensible à la casse).
    pub fn tag(&self, key: &str) -> Option<&str> {
        self.tags
            .iter()
            .find(|(candidate, _)| candidate.eq_ignore_ascii_case(key))
            .map(|(_, value)| value.as_str())
            .filter(|value| !value.is_empty())
    }

    /// Lien vers la VM dans le portail Azure (le tenant par défaut du compte
    /// connecté est utilisé).
    pub fn portal_url(&self) -> String {
        format!(
            "https://portal.azure.com/#@/resource/subscriptions/{}/resourceGroups/{}\
/providers/Microsoft.Compute/virtualMachines/{}/overview",
            percent_encode(&self.subscription_id),
            percent_encode(&self.resource_group),
            percent_encode(&self.name),
        )
    }

    pub fn is_running(&self) -> bool {
        self.power.as_deref() == Some("running")
    }

    /// Session de connexion ; la subscription est passée par identifiant, ce
    /// qui évite toute ambiguïté de nom.
    pub fn to_session(&self, prefer_private_ip: bool) -> AzureVmSession {
        AzureVmSession {
            subscription: Some(self.subscription_id.clone()),
            resource_group: self.resource_group.clone(),
            vm_name: self.name.clone(),
            prefer_private_ip,
        }
    }
}

/// Recherche saisie par l'utilisateur : `CLE=VALEUR` filtre sur un tag
/// (exact, insensible à la casse), les autres mots cherchent un fragment
/// dans le nom, le groupe, la région, l'état et les valeurs de tags affichés.
#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Query {
    tag_filters: Vec<(String, String)>,
    words: Vec<String>,
}

impl Query {
    pub fn parse(input: &str) -> Self {
        let mut normalized = input.to_owned();
        // « Application = Gitlab » → « Application=Gitlab ».
        while normalized.contains(" =") || normalized.contains("= ") {
            normalized = normalized.replace(" =", "=").replace("= ", "=");
        }
        let mut query = Self::default();
        for token in normalized.split_whitespace() {
            match token.split_once('=') {
                Some((key, value)) if !key.is_empty() => query
                    .tag_filters
                    .push((key.to_lowercase(), value.to_lowercase())),
                _ => query.words.push(token.to_lowercase()),
            }
        }
        query
    }

    pub fn matches(&self, row: &VmRow) -> bool {
        let tags_match = self.tag_filters.iter().all(|(key, value)| {
            row.tags
                .iter()
                .any(|(k, v)| k.to_lowercase() == *key && v.to_lowercase() == *value)
        });
        tags_match
            && self
                .words
                .iter()
                .all(|word| row.searchable_text().contains(word))
    }
}

impl VmRow {
    fn searchable_text(&self) -> String {
        let mut text = format!(
            "{} {} {} {}",
            self.name,
            self.resource_group,
            self.location,
            self.power.as_deref().unwrap_or("")
        );
        for key in ["Application", "Components", "Middlewares"] {
            if let Some(value) = self.tag(key) {
                text.push(' ');
                text.push_str(value);
            }
        }
        text.to_lowercase()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = r#"{
      "count": 3, "total_records": 3,
      "data": [
        {"subscriptionId": "sub-b", "resourceGroup": "Zeta-rg", "name": "vm2",
         "location": "francecentral", "power": "PowerState/deallocated",
         "tags": {"Application": "GITLAB", "environment": "dev", "Components": "web,os", "Middlewares": "apache"}},
        {"subscriptionId": "sub-a", "resourceGroup": "beta-rg", "name": "VM-b",
         "location": "francecentral", "power": "PowerState/running", "tags": null},
        {"subscriptionId": "sub-a", "resourceGroup": "Alpha-rg", "name": "vm1",
         "location": "westeurope", "power": "", "tags": {"Count": 3}}
      ]
    }"#;

    fn sample() -> VmList {
        parse_graph_output(SAMPLE).unwrap()
    }

    #[test]
    fn rows_are_sorted_by_subscription_then_resource_group_then_name() {
        let names: Vec<_> = sample().rows.iter().map(|r| r.name.clone()).collect();
        assert_eq!(names, ["vm1", "VM-b", "vm2"]);
    }

    #[test]
    fn power_prefix_is_removed_and_empty_becomes_none() {
        let list = sample();
        assert_eq!(list.rows[0].power, None);
        assert_eq!(list.rows[1].power.as_deref(), Some("running"));
        assert_eq!(list.rows[2].power.as_deref(), Some("deallocated"));
        assert!(list.rows[1].is_running());
        assert!(!list.rows[2].is_running());
    }

    #[test]
    fn null_and_non_string_tags_are_tolerated() {
        let list = sample();
        assert!(list.rows[1].tags.is_empty());
        assert_eq!(list.rows[0].tag("count"), Some("3"));
    }

    #[test]
    fn tag_lookup_is_case_insensitive() {
        let row = &sample().rows[2];
        assert_eq!(row.tag("application"), Some("GITLAB"));
        assert_eq!(row.tag("Absent"), None);
    }

    #[test]
    fn truncation_is_detected() {
        let json = r#"{"total_records": 5, "data": []}"#;
        assert!(parse_graph_output(json).unwrap().truncated);
        assert!(!sample().truncated);
    }

    #[test]
    fn invalid_json_is_an_error() {
        assert!(parse_graph_output("pas du json").is_err());
    }

    #[test]
    fn query_tag_filters_are_exact_and_case_insensitive() {
        let list = sample();
        let matching = |input: &str| -> Vec<String> {
            let query = Query::parse(input);
            list.rows
                .iter()
                .filter(|row| query.matches(row))
                .map(|row| row.name.clone())
                .collect()
        };
        assert_eq!(matching("Application=gitlab"), ["vm2"]);
        assert_eq!(matching("application = Gitlab Environment=DEV"), ["vm2"]);
        assert!(matching("Application=git").is_empty());
        assert!(matching("Environment=prod").is_empty());
        assert_eq!(matching("").len(), 3);
    }

    #[test]
    fn query_words_search_displayed_fields() {
        let list = sample();
        let count = |input: &str| {
            let query = Query::parse(input);
            list.rows.iter().filter(|row| query.matches(row)).count()
        };
        assert_eq!(count("apache"), 1);
        assert_eq!(count("francecentral running"), 1);
        assert_eq!(count("alpha"), 1);
        assert_eq!(count("westeurope Application=gitlab"), 0);
    }

    #[test]
    fn graph_argv_scopes_to_subscription_and_keeps_query_as_one_argument() {
        let scoped = graph_query_argv(Some("sub-a"));
        let position = scoped.iter().position(|a| a == "--subscriptions").unwrap();
        assert_eq!(scoped[position + 1], "sub-a");
        assert_eq!(scoped.last().unwrap(), GRAPH_QUERY);
        assert!(!graph_query_argv(None).contains(&"--subscriptions".to_owned()));
    }

    #[test]
    fn session_uses_subscription_id() {
        let session = sample().rows[0].to_session(true);
        assert_eq!(session.subscription.as_deref(), Some("sub-a"));
        assert_eq!(session.resource_group, "Alpha-rg");
        assert!(session.prefer_private_ip);
    }

    #[test]
    fn portal_url_points_to_the_vm_and_encodes_segments() {
        let mut row = sample().rows[0].clone();
        assert_eq!(
            row.portal_url(),
            "https://portal.azure.com/#@/resource/subscriptions/sub-a/resourceGroups/Alpha-rg\
/providers/Microsoft.Compute/virtualMachines/vm1/overview"
        );
        row.resource_group = "rg/../x y?".to_owned();
        let url = row.portal_url();
        assert!(
            url.contains("/resourceGroups/rg%2F..%2Fx%20y%3F/providers/"),
            "{url}"
        );
    }

    #[test]
    fn start_argv_targets_the_vm() {
        let argv = start_vm_argv(&sample().rows[0]);
        assert_eq!(
            argv,
            [
                "az",
                "vm",
                "start",
                "--subscription",
                "sub-a",
                "--resource-group",
                "Alpha-rg",
                "--name",
                "vm1"
            ]
        );
    }
}
