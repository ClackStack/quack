use quack_protocol::CompatKind;

use super::*;
use crate::compat;

impl Engine {
    pub(super) fn replace_config(&mut self, config: Arc<Loaded>) {
        self.extension_config = self.imports.config(&config);
        self.config = config;
        self.config_tx.send_replace(self.config.clone());
    }

    pub(super) async fn import_compat(&mut self, kind: CompatKind, path: String, save: bool) {
        let base = self.config.clone();
        let effective = self.extension_config.clone();
        let selected = tokio::task::spawn_blocking(move || {
            let not_found = || {
                "That folder is no longer available. Run /reload, then review /compat again."
                    .to_owned()
            };
            let candidate = compat::discover(&effective)
                .into_iter()
                .find(|c| c.folder.kind == kind && c.folder.path == path)
                .ok_or_else(not_found)?;
            let saved = if save {
                let fresh = base.reload().map_err(|e| e.to_string())?;
                let current = compat::discover(&fresh)
                    .into_iter()
                    .find(|c| {
                        c.folder.kind == kind
                            && c.folder.path == path
                            && c.folder.scope == candidate.folder.scope
                    })
                    .ok_or_else(not_found)?;
                let file = current.save(&fresh)?;
                let note = match fresh.reload() {
                    Ok(cfg) if current.is_configured(&cfg) => None,
                    Ok(_) => Some(
                        "Another setting overrides this path. Loaded for this run only.".to_owned(),
                    ),
                    Err(_) => Some(
                        "Could not verify the saved setting. Run /reload to check it.".to_owned(),
                    ),
                };
                Some((file, note))
            } else {
                None
            };
            Ok::<_, String>((candidate, saved))
        })
        .await;
        let (candidate, saved) = match selected {
            Ok(Ok(result)) => result,
            Ok(Err(message)) => return self.warn(message),
            Err(_) => {
                return self
                    .warn("Could not open the folder. Review /compat and try again.".into());
            }
        };
        let path = candidate.folder.path.clone();
        let persisted = saved.as_ref().is_some_and(|(_, note)| note.is_none());
        self.imports.enable(&self.config, candidate, persisted);
        self.extension_config = self.imports.config(&self.config);
        self.refresh_imports().await;
        if let Some((file, note)) = saved {
            self.emit(Event::Notice {
                message: format!("Loaded {path} and saved its path to {}", file.display()),
            });
            if let Some(note) = note {
                self.warn(note);
            }
        } else {
            self.emit(Event::Notice {
                message: format!("Loaded {path} for this run"),
            });
        }
    }

    async fn refresh_imports(&mut self) {
        let config = self.extension_config.clone();
        let tx = self.tx.clone();
        let setups = self
            .convo
            .as_ref()
            .map(|c| (c.setup.clone(), c.editor.clone()));
        let refreshed = tokio::task::spawn_blocking(move || {
            let setups = if let Some((active, editor)) = setups {
                let active = startup::refresh_extensions(&active, &config, &tx);
                let editor = editor.map(|old| {
                    let (quiet, _) = broadcast::channel(16);
                    Arc::new(startup::refresh_extensions(&old, &config, &quiet))
                });
                Some((active, editor))
            } else {
                startup::report_skills(&tx, &crate::env::skills(&config));
                let (commands, warnings) = crate::env::commands(&config);
                startup::report_commands(&tx, &commands, &warnings);
                None
            };
            compat::report(&tx, compat::discover(&config));
            setups
        })
        .await;
        if let (Ok(Some((fresh, editor))), Some(c)) = (refreshed, &mut self.convo) {
            if fresh.system != c.setup.system
                || fresh.effective_session_context() != c.setup.effective_session_context()
                || fresh.surface.tools != c.setup.surface.tools
            {
                c.responses_session = Arc::default();
            }
            self.skills_changed |= fresh.vars.skills != c.setup.vars.skills;
            c.setup = Arc::new(fresh);
            c.editor = editor;
        }
    }
}
