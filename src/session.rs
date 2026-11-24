//! Modèle de session et construction des commandes.
//!
//! Aucun champ secret : l'authentification passe par ssh-agent, les clés
//! (`identity_file`) ou la session `az login`.

use serde::{Deserialize, Serialize};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Session {
    pub id: u64,
    pub name: String,
    /// Dossier d'affichage (ex. « Prod/Front »), `None` = racine.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub folder: Option<String>,
    /// Couleur d'identification `#rrggbb` (pastille, bandeau du terminal).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    /// Palette du terminal propre à la session (identifiant de `palette`).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub palette: Option<String>,
    #[serde(flatten)]
    pub kind: SessionKind,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SessionKind {
    Ssh(SshSession),
    AzureVm(AzureVmSession),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SshSession {
    /// Hôte ou alias de `~/.ssh/config` (ssh applique alors sa propre config).
    pub host: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub user: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub port: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub identity_file: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub jump_host: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AzureVmSession {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub subscription: Option<String>,
    pub resource_group: String,
    pub vm_name: String,
    #[serde(default)]
    pub prefer_private_ip: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ValidationError {
    Empty(&'static str),
    InvalidValue(&'static str),
}

impl fmt::Display for ValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty(field) => write!(formatter, "le champ « {field} » est vide"),
            Self::InvalidValue(field) => write!(formatter, "valeur invalide pour « {field} »"),
        }
    }
}

impl std::error::Error for ValidationError {}

/// Refuse les valeurs vides, débutant par `-` (injection d'option), ou
/// contenant espaces/caractères de contrôle.
fn check_token(field: &'static str, value: &str) -> Result<(), ValidationError> {
    if value.is_empty() {
        return Err(ValidationError::Empty(field));
    }
    if value.starts_with('-') || value.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(ValidationError::InvalidValue(field));
    }
    Ok(())
}

impl Session {
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.name.trim().is_empty() {
            return Err(ValidationError::Empty("nom"));
        }
        // La couleur est injectée dans du CSS : seul `#rrggbb` est accepté.
        if let Some(color) = &self.color
            && crate::palette::parse_hex(color).is_none()
        {
            return Err(ValidationError::InvalidValue("couleur"));
        }
        if let Some(palette) = &self.palette
            && crate::palette::by_id(palette).is_none()
        {
            return Err(ValidationError::InvalidValue("palette"));
        }
        match &self.kind {
            SessionKind::Ssh(ssh) => ssh.validate(),
            SessionKind::AzureVm(vm) => vm.validate(),
        }
    }

    /// Résumé court pour l'affichage (hôte ou groupe de ressources / VM).
    pub fn summary(&self) -> String {
        match &self.kind {
            SessionKind::Ssh(ssh) => {
                let user = ssh
                    .user
                    .as_deref()
                    .map(|u| format!("{u}@"))
                    .unwrap_or_default();
                let port = ssh.port.map(|p| format!(":{p}")).unwrap_or_default();
                format!("{user}{}{port}", ssh.host)
            }
            SessionKind::AzureVm(vm) => format!("Azure · {} / {}", vm.resource_group, vm.vm_name),
        }
    }

    /// Commande à lancer, sous forme d'argv (jamais de `sh -c`).
    pub fn argv(&self) -> Result<Vec<String>, ValidationError> {
        self.validate()?;
        Ok(match &self.kind {
            SessionKind::Ssh(ssh) => ssh.argv(),
            SessionKind::AzureVm(vm) => vm.argv(),
        })
    }
}

impl SshSession {
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_token("hôte", &self.host)?;
        if let Some(user) = &self.user {
            check_token("utilisateur", user)?;
        }
        if self.port == Some(0) {
            return Err(ValidationError::InvalidValue("port"));
        }
        if let Some(jump_host) = &self.jump_host {
            check_token("rebond", jump_host)?;
        }
        if let Some(identity_file) = &self.identity_file {
            check_token("clé", identity_file)?;
        }
        Ok(())
    }

    fn argv(&self) -> Vec<String> {
        let mut argv = vec!["ssh".to_owned()];
        if let Some(port) = self.port {
            argv.extend(["-p".to_owned(), port.to_string()]);
        }
        if let Some(identity_file) = &self.identity_file {
            argv.extend(["-i".to_owned(), identity_file.clone()]);
        }
        if let Some(jump_host) = &self.jump_host {
            argv.extend(["-J".to_owned(), jump_host.clone()]);
        }
        argv.push("--".to_owned());
        argv.push(match &self.user {
            Some(user) => format!("{user}@{}", self.host),
            None => self.host.clone(),
        });
        argv
    }
}

impl AzureVmSession {
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_token("resource group", &self.resource_group)?;
        check_token("nom de VM", &self.vm_name)?;
        if let Some(subscription) = &self.subscription {
            // Un nom de subscription peut contenir des espaces.
            if subscription.is_empty() || subscription.starts_with('-') {
                return Err(ValidationError::InvalidValue("subscription"));
            }
        }
        Ok(())
    }

    fn argv(&self) -> Vec<String> {
        let mut argv: Vec<String> = ["az", "ssh", "vm"].map(str::to_owned).to_vec();
        if let Some(subscription) = &self.subscription {
            argv.extend(["--subscription".to_owned(), subscription.clone()]);
        }
        argv.extend([
            "--resource-group".to_owned(),
            self.resource_group.clone(),
            "--name".to_owned(),
            self.vm_name.clone(),
        ]);
        if self.prefer_private_ip {
            argv.push("--prefer-private-ip".to_owned());
        }
        argv
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ssh(host: &str) -> SshSession {
        SshSession {
            host: host.to_owned(),
            user: None,
            port: None,
            identity_file: None,
            jump_host: None,
        }
    }

    fn session(kind: SessionKind) -> Session {
        Session {
            id: 1,
            name: "test".to_owned(),
            folder: None,
            color: None,
            palette: None,
            kind,
        }
    }

    #[test]
    fn ssh_argv_minimal() {
        let argv = session(SessionKind::Ssh(ssh("example.com")))
            .argv()
            .unwrap();
        assert_eq!(argv, ["ssh", "--", "example.com"]);
    }

    #[test]
    fn ssh_argv_complete() {
        let mut full = ssh("10.0.0.4");
        full.user = Some("deploy".into());
        full.port = Some(2222);
        full.identity_file = Some("/home/dummy/.ssh/id_dummy".into());
        full.jump_host = Some("bastion".into());
        let argv = session(SessionKind::Ssh(full)).argv().unwrap();
        assert_eq!(
            argv,
            [
                "ssh",
                "-p",
                "2222",
                "-i",
                "/home/dummy/.ssh/id_dummy",
                "-J",
                "bastion",
                "--",
                "deploy@10.0.0.4"
            ]
        );
    }

    #[test]
    fn ssh_rejects_option_injection_and_whitespace() {
        for host in ["-oProxyCommand=evil", "a b", "", "host\n"] {
            assert!(
                session(SessionKind::Ssh(ssh(host))).argv().is_err(),
                "{host:?}"
            );
        }
        let mut bad_user = ssh("example.com");
        bad_user.user = Some("-l".into());
        assert!(session(SessionKind::Ssh(bad_user)).argv().is_err());
        let mut bad_port = ssh("example.com");
        bad_port.port = Some(0);
        assert!(session(SessionKind::Ssh(bad_port)).argv().is_err());
    }

    #[test]
    fn azure_argv_with_subscription_and_private_ip() {
        let vm = AzureVmSession {
            subscription: Some("Dummy Subscription".into()),
            resource_group: "dummy-rg".into(),
            vm_name: "dummy-vm".into(),
            prefer_private_ip: true,
        };
        let argv = session(SessionKind::AzureVm(vm)).argv().unwrap();
        assert_eq!(
            argv,
            [
                "az",
                "ssh",
                "vm",
                "--subscription",
                "Dummy Subscription",
                "--resource-group",
                "dummy-rg",
                "--name",
                "dummy-vm",
                "--prefer-private-ip"
            ]
        );
    }

    #[test]
    fn azure_rejects_invalid_values() {
        let vm = AzureVmSession {
            subscription: None,
            resource_group: "-g".into(),
            vm_name: "vm".into(),
            prefer_private_ip: false,
        };
        assert!(session(SessionKind::AzureVm(vm)).argv().is_err());
    }

    #[test]
    fn summary_formats() {
        let mut full = ssh("10.0.0.4");
        full.user = Some("deploy".into());
        full.port = Some(2222);
        assert_eq!(
            session(SessionKind::Ssh(full)).summary(),
            "deploy@10.0.0.4:2222"
        );
        let vm = AzureVmSession {
            subscription: None,
            resource_group: "dummy-rg".into(),
            vm_name: "dummy-vm".into(),
            prefer_private_ip: false,
        };
        assert_eq!(
            session(SessionKind::AzureVm(vm)).summary(),
            "Azure · dummy-rg / dummy-vm"
        );
    }

    #[test]
    fn color_and_palette_are_validated() {
        let mut custom = session(SessionKind::Ssh(ssh("example.com")));
        custom.color = Some("#e01b24".into());
        custom.palette = Some("nord".into());
        assert!(custom.validate().is_ok());

        for bad_color in ["red", "#12345", "#e01b24; } * { display: none", "url(x)"] {
            custom.color = Some(bad_color.into());
            assert_eq!(
                custom.validate(),
                Err(ValidationError::InvalidValue("couleur")),
                "{bad_color}"
            );
        }
        custom.color = None;
        custom.palette = Some("inconnue".into());
        assert_eq!(
            custom.validate(),
            Err(ValidationError::InvalidValue("palette"))
        );
    }

    #[test]
    fn old_files_without_color_or_palette_still_load() {
        let json = r#"{"id":1,"name":"a","type":"ssh","host":"example.com"}"#;
        let loaded: Session = serde_json::from_str(json).unwrap();
        assert_eq!((loaded.color, loaded.palette), (None, None));
    }

    #[test]
    fn empty_name_is_rejected() {
        let mut empty_name = session(SessionKind::Ssh(ssh("example.com")));
        empty_name.name = "  ".into();
        assert_eq!(empty_name.validate(), Err(ValidationError::Empty("nom")));
    }
}
