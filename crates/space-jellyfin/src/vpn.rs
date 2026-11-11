//! The Jellyfin space's WireGuard tunnel (ADR 0037): a server reachable only through a VPN. Its
//! configuration (a wg-quick file, keys included) lives in the Credential Manager
//! (`session::load_vpn`); while it is on, every request of the space's client and every video
//! go through the tunnel's local proxy. The settings import it, turn it on and off, and show
//! whether the server answers.

use std::path::PathBuf;
use std::time::Duration;

use gpui::prelude::*;
use gpui::{App, Context, PathPromptOptions, SharedString, Task, WeakEntity, Window, div, px};
use gpui_component::button::Button;
use gpui_component::{ActiveTheme, Sizable, h_flex, v_flex};
use peeby_core::settings::Settings;
use ui::form::{row, section, toggle};
use ui::i18n::t;
use ui::{runtime, tf, theme};
use wireguard::{Config, ConfigError, Tunnel};

use super::JellyfinSpace;

/// Without a handshake for this long, the VPN server is said not to answer.
const NO_ANSWER: Duration = Duration::from_secs(10);

/// A larger file is not a WireGuard configuration.
const MAX_FILE: u64 = 64 << 10;

/// An imported configuration, and its tunnel while it is on.
pub struct Vpn {
  config: Config,
  tunnel: Option<Tunnel>,
  /// Why the tunnel did not start.
  error: Option<String>,
}

impl Vpn {
  pub fn new(config: Config) -> Self {
    Self {
      config,
      tunnel: None,
      error: None,
    }
  }

  /// Starts the tunnel again (the old one stops first).
  pub fn start(&mut self, cx: &App) {
    self.tunnel = None;
    match Tunnel::start(self.config.clone(), &runtime::handle(cx)) {
      Ok(tunnel) => {
        self.tunnel = Some(tunnel);
        self.error = None;
      }
      Err(e) => {
        tracing::warn!("WireGuard: {e}");
        self.error = Some(e.to_string());
      }
    }
  }

  pub fn stop(&mut self) {
    self.tunnel = None;
  }

  /// Whether the session with the VPN server is open now (something used it lately): a request
  /// then costs no new connection.
  pub fn connected(&self) -> bool {
    self
      .tunnel
      .as_ref()
      .is_some_and(|tunnel| tunnel.status().connected.is_some())
  }

  /// The tunnel's local proxy while it runs. It holds a secret: never log it.
  pub fn proxy(&self) -> Option<&str> {
    self.tunnel.as_ref().map(Tunnel::proxy_url)
  }
}

/// What is wrong with a configuration file, for the user.
pub fn describe(error: &ConfigError) -> String {
  match error {
    ConfigError::Missing(key) => tf!(
      "This is not a WireGuard configuration: {key} is missing.",
      "Ce n'est pas une configuration WireGuard : {key} manque."
    ),
    ConfigError::Invalid(key) => tf!(
      "The configuration's {key} cannot be read.",
      "Le champ {key} de la configuration est illisible."
    ),
    ConfigError::SeveralPeers => t(
      "The configuration has several [Peer]: Peeby takes one.",
      "La configuration a plusieurs [Peer] : Peeby n'en prend qu'un.",
    )
    .to_owned(),
  }
}

/// The settings' "VPN" section.
pub struct VpnView {
  nav: WeakEntity<JellyfinSpace>,
  /// The last import's problem.
  error: Option<SharedString>,
  /// Redraws the status every second while it shows.
  _tick: Task<()>,
}

impl VpnView {
  pub fn new(nav: WeakEntity<JellyfinSpace>, cx: &mut Context<Self>) -> Self {
    cx.observe_global::<Settings>(|_, cx| cx.notify()).detach();
    Self {
      nav,
      error: None,
      _tick: Task::ready(()),
    }
  }

  fn pick_file(&mut self, cx: &mut Context<Self>) {
    let paths = cx.prompt_for_paths(PathPromptOptions {
      files: true,
      directories: false,
      multiple: false,
      prompt: Some(t("Import", "Importer").into()),
    });
    cx.spawn(async move |this, cx| {
      let Ok(Ok(Some(paths))) = paths.await else {
        return;
      };
      let Some(path) = paths.into_iter().next() else {
        return;
      };
      this.update(cx, |this, cx| this.import(path, cx)).ok();
    })
    .detach();
  }

  fn import(&mut self, path: PathBuf, cx: &mut Context<Self>) {
    let read = cx.background_executor().spawn(async move {
      let size = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
      if size > MAX_FILE {
        return Err(
          t(
            "This file is too large for a WireGuard configuration.",
            "Ce fichier est trop gros pour une configuration WireGuard.",
          )
          .to_owned(),
        );
      }
      std::fs::read_to_string(&path)
        .map_err(|e| tf!("Cannot read the file: {e}", "Lecture impossible : {e}"))
    });
    cx.spawn(async move |this, cx| {
      let text = read.await;
      this
        .update(cx, |this, cx| {
          let result = text.and_then(|text| {
            this
              .nav
              .update(cx, |space, cx| space.import_vpn(&text, cx))
              .map_err(|_| String::new())?
              .map_err(|e| describe(&e))
          });
          this.error = result.err().filter(|e| !e.is_empty()).map(Into::into);
          cx.notify();
        })
        .ok();
    })
    .detach();
  }

  /// The tunnel's state, in words.
  fn status(vpn: &Vpn, on: bool) -> String {
    if !on {
      return t("Off.", "Désactivé.").to_owned();
    }
    if let Some(e) = &vpn.error {
      return tf!("Cannot start: {e}", "Démarrage impossible : {e}");
    }
    let Some(tunnel) = &vpn.tunnel else {
      return t("Off.", "Désactivé.").to_owned();
    };
    let status = tunnel.status();
    let Some(connected) = status.connected else {
      let minutes = wireguard::IDLE.as_secs() / 60;
      return tf!(
        "On standby: connects when Jellyfin needs it, disconnects after {minutes} min unused.",
        "En veille : se connecte quand Jellyfin en a besoin, se déconnecte après {minutes} min \
         sans usage."
      );
    };
    match (status.handshake, status.error) {
      (Some(age), _) => {
        let age = ago(age);
        tf!(
          "Connected: the server answered {age}.",
          "Connecté : le serveur a répondu {age}."
        )
      }
      (None, Some(e)) => tf!(
        "The VPN server cannot be reached: {e}",
        "Le serveur VPN est injoignable : {e}"
      ),
      // A handshake takes a round trip: this long without one, the server is not answering.
      (None, None) if connected > NO_ANSWER => tf!(
        "The VPN server does not answer. Check its Endpoint (a name behind a proxy such as          Cloudflare's does not carry WireGuard), its UDP port and the keys.",
        "Le serveur VPN ne répond pas. Vérifiez son Endpoint (un nom derrière un proxy comme          celui de Cloudflare ne transporte pas WireGuard), son port UDP et les clés."
      ),
      (None, None) => t(
        "Waiting for the VPN server's answer…",
        "En attente de la réponse du serveur VPN…",
      )
      .to_owned(),
    }
  }
}

/// "12 s ago", "3 min ago".
fn ago(age: Duration) -> String {
  let seconds = age.as_secs();
  if seconds < 90 {
    tf!("{seconds} s ago", "il y a {seconds} s")
  } else {
    let minutes = seconds / 60;
    tf!("{minutes} min ago", "il y a {minutes} min")
  }
}

impl Render for VpnView {
  fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
    let on = Settings::get(cx).jellyfin.vpn;
    let nav = self.nav.clone();
    let space = self.nav.upgrade();
    let space = space.as_ref().map(|space| space.read(cx));
    let mut rows = Vec::new();
    let import = |id: &'static str, label: &'static str, cx: &mut Context<Self>| {
      Button::new(id)
        .small()
        .label(label)
        .on_click(cx.listener(|this, _, _, cx| this.pick_file(cx)))
    };
    match space.and_then(JellyfinSpace::vpn) {
      None => rows.push(
        row(
          "WireGuard",
          t(
            "For a server reachable only through a WireGuard VPN: import its configuration \
             file (.conf), and Peeby reaches the server through it, without a driver or \
             administrator rights.",
            "Pour un serveur joignable seulement par un VPN WireGuard : importez son fichier \
             de configuration (.conf), et Peeby passe par lui pour joindre le serveur, sans \
             pilote ni droits d'administrateur.",
          ),
          import("jellyfin-vpn-import", t("Import…", "Importer…"), cx),
        )
        .into_any_element(),
      ),
      Some(vpn) => {
        rows.push(
          row(
            "WireGuard",
            t(
              "Every request to the server and every video go through the tunnel. Turning \
               it on or off reloads the Jellyfin pages.",
              "Chaque requête au serveur et chaque vidéo passent par le tunnel. L'activer ou \
               le désactiver recharge les pages Jellyfin.",
            ),
            toggle("jellyfin-vpn", on, move |on, cx| {
              nav
                .update(cx, |space, cx| space.set_vpn_enabled(on, cx))
                .ok();
            }),
          )
          .into_any_element(),
        );
        let routes = vpn.config.allowed_ips();
        let line = |label: &str, value: String| {
          h_flex()
            .gap_4()
            .child(
              div()
                .w(px(120.))
                .flex_none()
                .text_sm()
                .text_color(theme::muted())
                .child(label.to_owned()),
            )
            .child(div().text_sm().min_w_0().truncate().child(value))
        };
        let nav = self.nav.clone();
        rows.push(
          v_flex()
            .px_5()
            .py_4()
            .gap_2()
            .rounded(theme::RADIUS)
            .bg(theme::surface())
            .child(line(t("Status", "État"), Self::status(vpn, on)))
            .child(line(
              t("VPN server", "Serveur VPN"),
              // With the address its name resolved to: what the handshakes go to.
              match vpn.tunnel.as_ref().and_then(|t| t.status().endpoint) {
                Some(address) => format!("{} ({})", vpn.config.endpoint(), address.ip()),
                None => vpn.config.endpoint().to_owned(),
              },
            ))
            .child(line(
              t("Address", "Adresse"),
              vpn.config.addresses().join(", "),
            ))
            .children(
              (!vpn.config.dns().is_empty())
                .then(|| line(t("DNS", "DNS"), vpn.config.dns().join(", "))),
            )
            .child(line(
              t("Routed", "Routé"),
              if routes.is_empty() {
                t(
                  "Nothing (AllowedIPs is empty)",
                  "Rien (AllowedIPs est vide)",
                )
                .to_owned()
              } else {
                routes.join(", ")
              },
            ))
            .child(
              h_flex()
                .pt_2()
                .gap_2()
                .child(import(
                  "jellyfin-vpn-replace",
                  t("Replace…", "Remplacer…"),
                  cx,
                ))
                .child(
                  Button::new("jellyfin-vpn-remove")
                    .small()
                    .label(t("Remove", "Supprimer"))
                    .on_click(move |_, _, cx| {
                      nav.update(cx, |space, cx| space.remove_vpn(cx)).ok();
                    }),
                ),
            )
            .into_any_element(),
        );
        // The status follows the handshakes while it shows.
        if on {
          self._tick = cx.spawn(async move |this, cx| {
            cx.background_executor().timer(Duration::from_secs(1)).await;
            this.update(cx, |_, cx| cx.notify()).ok();
          });
        }
      }
    }
    if let Some(error) = self.error.clone() {
      rows.push(
        div()
          .text_sm()
          .text_color(cx.theme().danger)
          .child(error)
          .into_any_element(),
      );
    }
    section(t("VPN", "VPN"), rows)
  }
}
