use super::{ChannelsConfig, KernelConfig, SearchProvider, WebConfig};

impl KernelConfig {
    /// Validate the configuration, returning a list of warnings.
    ///
    /// Checks that env vars referenced by configured channels or the selected
    /// search provider are set. This is warning-only validation: loading must
    /// stay resilient, and operators get actionable messages instead.
    pub fn validate(&self) -> Vec<String> {
        let mut warnings = Vec::new();
        warn_primary_channel_envs(&mut warnings, &self.channels);
        warn_wave_three_channel_envs(&mut warnings, &self.channels);
        warn_wave_four_channel_envs(&mut warnings, &self.channels);
        warn_wave_five_channel_envs(&mut warnings, &self.channels);
        warn_search_provider_envs(&mut warnings, &self.web);

        warnings
    }

    /// Clamp configuration values to safe production bounds.
    ///
    /// Called after loading config to prevent zero timeouts, unbounded buffers,
    /// or other misconfigurations that cause silent failures at runtime.
    pub fn clamp_bounds(&mut self) {
        if self.browser.timeout_secs == 0 {
            self.browser.timeout_secs = 30;
        } else if self.browser.timeout_secs > 300 {
            self.browser.timeout_secs = 300;
        }

        if self.browser.max_sessions == 0 {
            self.browser.max_sessions = 3;
        } else if self.browser.max_sessions > 100 {
            self.browser.max_sessions = 100;
        }

        if self.web.fetch.max_response_bytes == 0 {
            self.web.fetch.max_response_bytes = 5_000_000;
        } else if self.web.fetch.max_response_bytes > 50_000_000 {
            self.web.fetch.max_response_bytes = 50_000_000;
        }

        if self.web.fetch.timeout_secs == 0 {
            self.web.fetch.timeout_secs = 30;
        } else if self.web.fetch.timeout_secs > 120 {
            self.web.fetch.timeout_secs = 120;
        }
    }
}

fn warn_primary_channel_envs(warnings: &mut Vec<String>, channels: &ChannelsConfig) {
    if let Some(tg) = &channels.telegram {
        warn_missing(warnings, "Telegram", &tg.bot_token_env);
    }
    if let Some(dc) = &channels.discord {
        warn_missing(warnings, "Discord", &dc.bot_token_env);
    }
    if let Some(sl) = &channels.slack {
        warn_missing(warnings, "Slack", &sl.app_token_env);
        warn_missing(warnings, "Slack", &sl.bot_token_env);
    }
    if let Some(wa) = &channels.whatsapp {
        warn_missing(warnings, "WhatsApp", &wa.access_token_env);
    }
    if let Some(mx) = &channels.matrix {
        warn_missing(warnings, "Matrix", &mx.access_token_env);
    }
    if let Some(em) = &channels.email {
        warn_missing(warnings, "Email", &em.password_env);
    }
    if let Some(t) = &channels.teams {
        warn_missing(warnings, "Teams", &t.app_password_env);
    }
    if let Some(m) = &channels.mattermost {
        warn_missing(warnings, "Mattermost", &m.token_env);
    }
    if let Some(z) = &channels.zulip {
        warn_missing(warnings, "Zulip", &z.api_key_env);
    }
    if let Some(tw) = &channels.twitch {
        warn_missing(warnings, "Twitch", &tw.oauth_token_env);
    }
    if let Some(rc) = &channels.rocketchat {
        warn_missing(warnings, "Rocket.Chat", &rc.token_env);
    }
    if let Some(gc) = &channels.google_chat {
        warn_missing(warnings, "Google Chat", &gc.service_account_env);
    }
    if let Some(x) = &channels.xmpp {
        warn_missing(warnings, "XMPP", &x.password_env);
    }
}

fn warn_wave_three_channel_envs(warnings: &mut Vec<String>, channels: &ChannelsConfig) {
    if let Some(ln) = &channels.line {
        warn_missing(warnings, "LINE", &ln.access_token_env);
    }
    if let Some(vb) = &channels.viber {
        warn_missing(warnings, "Viber", &vb.auth_token_env);
    }
    if let Some(ms) = &channels.messenger {
        warn_missing(warnings, "Messenger", &ms.page_token_env);
    }
    if let Some(rd) = &channels.reddit {
        warn_missing(warnings, "Reddit", &rd.client_secret_env);
    }
    if let Some(md) = &channels.mastodon {
        warn_missing(warnings, "Mastodon", &md.access_token_env);
    }
    if let Some(bs) = &channels.bluesky {
        warn_missing(warnings, "Bluesky", &bs.app_password_env);
    }
    if let Some(fs) = &channels.feishu {
        warn_missing(warnings, "Feishu", &fs.app_secret_env);
    }
    if let Some(rv) = &channels.revolt {
        warn_missing(warnings, "Revolt", &rv.bot_token_env);
    }
}

fn warn_wave_four_channel_envs(warnings: &mut Vec<String>, channels: &ChannelsConfig) {
    if let Some(nc) = &channels.nextcloud {
        warn_missing(warnings, "Nextcloud", &nc.token_env);
    }
    if let Some(gd) = &channels.guilded {
        warn_missing(warnings, "Guilded", &gd.bot_token_env);
    }
    if let Some(kb) = &channels.keybase {
        warn_missing(warnings, "Keybase", &kb.paperkey_env);
    }
    if let Some(tm) = &channels.threema {
        warn_missing(warnings, "Threema", &tm.secret_env);
    }
    if let Some(ns) = &channels.nostr {
        warn_missing(warnings, "Nostr", &ns.private_key_env);
    }
    if let Some(wx) = &channels.webex {
        warn_missing(warnings, "Webex", &wx.bot_token_env);
    }
    if let Some(pb) = &channels.pumble {
        warn_missing(warnings, "Pumble", &pb.bot_token_env);
    }
    if let Some(fl) = &channels.flock {
        warn_missing(warnings, "Flock", &fl.bot_token_env);
    }
    if let Some(tw) = &channels.twist {
        warn_missing(warnings, "Twist", &tw.token_env);
    }
}

fn warn_wave_five_channel_envs(warnings: &mut Vec<String>, channels: &ChannelsConfig) {
    if let Some(mb) = &channels.mumble {
        warn_missing(warnings, "Mumble", &mb.password_env);
    }
    if let Some(dt) = &channels.dingtalk {
        warn_missing(warnings, "DingTalk", &dt.access_token_env);
    }
    if let Some(ds) = &channels.dingtalk_stream {
        warn_missing(warnings, "DingTalk Stream", &ds.app_key_env);
        warn_missing(warnings, "DingTalk Stream", &ds.app_secret_env);
    }
    if let Some(dc) = &channels.discourse {
        warn_missing(warnings, "Discourse", &dc.api_key_env);
    }
    if let Some(gt) = &channels.gitter {
        warn_missing(warnings, "Gitter", &gt.token_env);
    }
    if let Some(nf) = &channels.ntfy {
        if !nf.token_env.is_empty() {
            warn_missing(warnings, "ntfy", &nf.token_env);
        }
    }
    if let Some(gf) = &channels.gotify {
        warn_missing(warnings, "Gotify", &gf.app_token_env);
    }
    if let Some(wh) = &channels.webhook {
        warn_missing(warnings, "Webhook", &wh.secret_env);
    }
    if let Some(li) = &channels.linkedin {
        warn_missing(warnings, "LinkedIn", &li.access_token_env);
    }
}

fn warn_search_provider_envs(warnings: &mut Vec<String>, web: &WebConfig) {
    match web.search_provider {
        SearchProvider::Brave => warn_search_missing(warnings, "Brave", &web.brave.api_key_env),
        SearchProvider::Tavily => warn_search_missing(warnings, "Tavily", &web.tavily.api_key_env),
        SearchProvider::Perplexity => {
            warn_search_missing(warnings, "Perplexity", &web.perplexity.api_key_env)
        }
        SearchProvider::DuckDuckGo | SearchProvider::Auto => {}
    }
}

fn warn_missing(warnings: &mut Vec<String>, label: &str, env_var: &str) {
    if env_var_missing(env_var) {
        warnings.push(format!("{label} configured but {env_var} is not set"));
    }
}

fn warn_search_missing(warnings: &mut Vec<String>, provider: &str, env_var: &str) {
    if env_var_missing(env_var) {
        warnings.push(format!(
            "{provider} search selected but {env_var} is not set"
        ));
    }
}

fn env_var_missing(env_var: &str) -> bool {
    std::env::var(env_var).unwrap_or_default().is_empty()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::{
        DingTalkStreamConfig, DiscordConfig, LineConfig, NextcloudConfig, NtfyConfig, SlackConfig,
        TelegramConfig,
    };

    #[test]
    fn validation_reports_configured_channel_and_search_envs() {
        let discord_env = "CAPTAIN_TEST_VALIDATION_DISCORD_TOKEN";
        let brave_env = "CAPTAIN_TEST_VALIDATION_BRAVE_KEY";
        unsafe {
            std::env::remove_var(discord_env);
            std::env::remove_var(brave_env);
        }

        let mut config = KernelConfig::default();
        config.channels.discord = Some(DiscordConfig {
            bot_token_env: discord_env.to_string(),
            ..Default::default()
        });
        config.web.search_provider = SearchProvider::Brave;
        config.web.brave.api_key_env = brave_env.to_string();

        let warnings = config.validate();

        assert!(warnings.contains(&format!("Discord configured but {discord_env} is not set")));
        assert!(warnings.contains(&format!("Brave search selected but {brave_env} is not set")));
    }

    #[test]
    fn validation_keeps_ntfy_token_optional_when_env_name_is_empty() {
        let mut config = KernelConfig::default();
        config.channels.ntfy = Some(NtfyConfig {
            token_env: String::new(),
            ..Default::default()
        });

        let warnings = config.validate();

        assert!(
            !warnings
                .iter()
                .any(|warning| warning.contains("ntfy configured")),
            "empty ntfy token_env is explicitly optional"
        );
    }

    #[test]
    fn validation_preserves_channel_warning_order() {
        let envs = [
            "CAPTAIN_TEST_VALIDATION_ORDER_TELEGRAM",
            "CAPTAIN_TEST_VALIDATION_ORDER_SLACK_APP",
            "CAPTAIN_TEST_VALIDATION_ORDER_SLACK_BOT",
            "CAPTAIN_TEST_VALIDATION_ORDER_LINE",
            "CAPTAIN_TEST_VALIDATION_ORDER_NEXTCLOUD",
            "CAPTAIN_TEST_VALIDATION_ORDER_DINGTALK_KEY",
            "CAPTAIN_TEST_VALIDATION_ORDER_DINGTALK_SECRET",
        ];
        unsafe {
            for env in envs {
                std::env::remove_var(env);
            }
        }

        let mut config = KernelConfig::default();
        config.channels.telegram = Some(TelegramConfig {
            bot_token_env: envs[0].to_string(),
            ..Default::default()
        });
        config.channels.slack = Some(SlackConfig {
            app_token_env: envs[1].to_string(),
            bot_token_env: envs[2].to_string(),
            ..Default::default()
        });
        config.channels.line = Some(LineConfig {
            access_token_env: envs[3].to_string(),
            ..Default::default()
        });
        config.channels.nextcloud = Some(NextcloudConfig {
            token_env: envs[4].to_string(),
            ..Default::default()
        });
        config.channels.dingtalk_stream = Some(DingTalkStreamConfig {
            app_key_env: envs[5].to_string(),
            app_secret_env: envs[6].to_string(),
            ..Default::default()
        });

        let warnings = config.validate();

        assert_eq!(
            warnings,
            vec![
                format!("Telegram configured but {} is not set", envs[0]),
                format!("Slack configured but {} is not set", envs[1]),
                format!("Slack configured but {} is not set", envs[2]),
                format!("LINE configured but {} is not set", envs[3]),
                format!("Nextcloud configured but {} is not set", envs[4]),
                format!("DingTalk Stream configured but {} is not set", envs[5]),
                format!("DingTalk Stream configured but {} is not set", envs[6]),
            ]
        );
    }

    #[test]
    fn clamp_bounds_preserves_runtime_safety_limits() {
        let mut config = KernelConfig::default();
        config.browser.timeout_secs = 999;
        config.browser.max_sessions = 0;
        config.web.fetch.max_response_bytes = 99_000_000;
        config.web.fetch.timeout_secs = 0;

        config.clamp_bounds();

        assert_eq!(config.browser.timeout_secs, 300);
        assert_eq!(config.browser.max_sessions, 3);
        assert_eq!(config.web.fetch.max_response_bytes, 50_000_000);
        assert_eq!(config.web.fetch.timeout_secs, 30);
    }
}
