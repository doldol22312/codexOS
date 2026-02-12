use core::str;

use crate::{fs, net, timer};

pub const CONFIG_FILE: &str = "discord.cfg";
const CONFIG_MAX_BYTES: usize = 2048;
const TOKEN_MAX_LEN: usize = 192;
const ID_MAX_LEN: usize = 40;
const ERROR_MAX_LEN: usize = 96;
const HEARTBEAT_TICKS: u32 = 100;
const DEFAULT_POLL_TICKS: u32 = 120;
const MIN_POLL_TICKS: u32 = 20;
const MAX_POLL_TICKS: u32 = 2_000;
const DEFAULT_BRIDGE_PORT: u16 = 4242;
const BRIDGE_TIMEOUT_TICKS: u32 = 280;
const BRIDGE_RETRY_TICKS: u32 = 120;
const BRIDGE_REQ_MAX_BYTES: usize = 512;
const BRIDGE_RESP_MAX_BYTES: usize = 1200;

const DEFAULT_BRIDGE_IP: net::Ipv4Addr = net::Ipv4Addr::new(10, 0, 2, 2);

pub const MAX_GUILDS: usize = 6;
pub const MAX_CHANNELS: usize = 12;
pub const MAX_MESSAGES: usize = 48;
const NAME_MAX_LEN: usize = 40;
const MESSAGE_MAX_LEN: usize = 160;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscordState {
    MissingConfig,
    MissingToken,
    Ready,
    Error,
}

impl DiscordState {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::MissingConfig => "missing-config",
            Self::MissingToken => "missing-token",
            Self::Ready => "ready",
            Self::Error => "error",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiscordError {
    Fs(fs::FsError),
    Net(net::NetError),
    MissingConfig,
    MissingToken,
    InvalidConfig,
    InvalidResponse,
    TokenTooLong,
    NotReady,
    Busy,
    MessageTooLong,
    BridgeRejected,
}

impl DiscordError {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Fs(error) => error.as_str(),
            Self::Net(error) => error.as_str(),
            Self::MissingConfig => "discord.cfg not found",
            Self::MissingToken => "bot_token missing in discord.cfg",
            Self::InvalidConfig => "invalid discord.cfg",
            Self::InvalidResponse => "invalid bridge response",
            Self::TokenTooLong => "bot token too long",
            Self::NotReady => "discord client is not ready",
            Self::Busy => "discord bridge busy",
            Self::MessageTooLong => "message too long",
            Self::BridgeRejected => "bridge rejected request",
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BridgeRequestKind {
    None,
    Sync,
    Send,
}

#[derive(Clone, Copy)]
pub struct DiscordConfig {
    token: [u8; TOKEN_MAX_LEN],
    token_len: usize,
    default_guild_id: [u8; ID_MAX_LEN],
    default_guild_id_len: usize,
    default_channel_id: [u8; ID_MAX_LEN],
    default_channel_id_len: usize,
    bridge_ip: net::Ipv4Addr,
    bridge_port: u16,
    poll_interval_ticks: u32,
}

impl DiscordConfig {
    pub const fn empty() -> Self {
        Self {
            token: [0; TOKEN_MAX_LEN],
            token_len: 0,
            default_guild_id: [0; ID_MAX_LEN],
            default_guild_id_len: 0,
            default_channel_id: [0; ID_MAX_LEN],
            default_channel_id_len: 0,
            bridge_ip: DEFAULT_BRIDGE_IP,
            bridge_port: DEFAULT_BRIDGE_PORT,
            poll_interval_ticks: DEFAULT_POLL_TICKS,
        }
    }

    pub fn token_len(&self) -> usize {
        self.token_len
    }

    pub fn token(&self) -> &str {
        str::from_utf8(&self.token[..self.token_len]).unwrap_or("")
    }

    pub fn default_guild_id(&self) -> &str {
        str::from_utf8(&self.default_guild_id[..self.default_guild_id_len]).unwrap_or("")
    }

    pub fn default_channel_id(&self) -> &str {
        str::from_utf8(&self.default_channel_id[..self.default_channel_id_len]).unwrap_or("")
    }

    pub const fn bridge_ip(&self) -> net::Ipv4Addr {
        self.bridge_ip
    }

    pub const fn bridge_port(&self) -> u16 {
        self.bridge_port
    }

    pub const fn poll_interval_ticks(&self) -> u32 {
        self.poll_interval_ticks
    }
}

#[derive(Clone, Copy)]
pub struct GuildSummary {
    pub id: [u8; ID_MAX_LEN],
    pub id_len: usize,
    pub name: [u8; NAME_MAX_LEN],
    pub name_len: usize,
}

impl GuildSummary {
    const fn empty() -> Self {
        Self {
            id: [0; ID_MAX_LEN],
            id_len: 0,
            name: [0; NAME_MAX_LEN],
            name_len: 0,
        }
    }

    pub fn id_str(&self) -> &str {
        str::from_utf8(&self.id[..self.id_len]).unwrap_or("")
    }

    pub fn name_str(&self) -> &str {
        str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    fn set(&mut self, id: &str, name: &str) {
        self.id_len = copy_ascii(self.id.as_mut_slice(), id.as_bytes());
        self.name_len = copy_ascii(self.name.as_mut_slice(), name.as_bytes());
    }

    fn set_bytes(&mut self, id: &[u8], name: &[u8]) {
        self.id_len = copy_ascii(self.id.as_mut_slice(), id);
        self.name_len = copy_ascii(self.name.as_mut_slice(), name);
    }
}

#[derive(Clone, Copy)]
pub struct ChannelSummary {
    pub id: [u8; ID_MAX_LEN],
    pub id_len: usize,
    pub name: [u8; NAME_MAX_LEN],
    pub name_len: usize,
}

impl ChannelSummary {
    const fn empty() -> Self {
        Self {
            id: [0; ID_MAX_LEN],
            id_len: 0,
            name: [0; NAME_MAX_LEN],
            name_len: 0,
        }
    }

    pub fn id_str(&self) -> &str {
        str::from_utf8(&self.id[..self.id_len]).unwrap_or("")
    }

    pub fn name_str(&self) -> &str {
        str::from_utf8(&self.name[..self.name_len]).unwrap_or("")
    }

    fn set(&mut self, id: &str, name: &str) {
        self.id_len = copy_ascii(self.id.as_mut_slice(), id.as_bytes());
        self.name_len = copy_ascii(self.name.as_mut_slice(), name.as_bytes());
    }

    fn set_bytes(&mut self, id: &[u8], name: &[u8]) {
        self.id_len = copy_ascii(self.id.as_mut_slice(), id);
        self.name_len = copy_ascii(self.name.as_mut_slice(), name);
    }
}

#[derive(Clone, Copy)]
pub struct MessageSummary {
    pub author: [u8; NAME_MAX_LEN],
    pub author_len: usize,
    pub content: [u8; MESSAGE_MAX_LEN],
    pub content_len: usize,
    pub local_echo: bool,
}

impl MessageSummary {
    const fn empty() -> Self {
        Self {
            author: [0; NAME_MAX_LEN],
            author_len: 0,
            content: [0; MESSAGE_MAX_LEN],
            content_len: 0,
            local_echo: false,
        }
    }

    pub fn author_str(&self) -> &str {
        str::from_utf8(&self.author[..self.author_len]).unwrap_or("")
    }

    pub fn content_str(&self) -> &str {
        str::from_utf8(&self.content[..self.content_len]).unwrap_or("")
    }

    fn set(&mut self, author: &str, content: &[u8], local_echo: bool) {
        self.author_len = copy_ascii(self.author.as_mut_slice(), author.as_bytes());
        self.content_len = copy_ascii(self.content.as_mut_slice(), content);
        self.local_echo = local_echo;
    }

    fn set_bytes(&mut self, author: &[u8], content: &[u8], local_echo: bool) {
        self.author_len = copy_ascii(self.author.as_mut_slice(), author);
        self.content_len = copy_ascii(self.content.as_mut_slice(), content);
        self.local_echo = local_echo;
    }
}

pub struct DiscordUiSnapshot {
    pub state: DiscordState,
    pub guilds: [GuildSummary; MAX_GUILDS],
    pub guild_count: usize,
    pub channels: [ChannelSummary; MAX_CHANNELS],
    pub channel_count: usize,
    pub messages: [MessageSummary; MAX_MESSAGES],
    pub message_count: usize,
    pub selected_guild: usize,
    pub selected_channel: usize,
    pub token_configured: bool,
    pub heartbeat_count: u32,
}

impl DiscordUiSnapshot {
    pub const fn empty() -> Self {
        Self {
            state: DiscordState::MissingConfig,
            guilds: [GuildSummary::empty(); MAX_GUILDS],
            guild_count: 0,
            channels: [ChannelSummary::empty(); MAX_CHANNELS],
            channel_count: 0,
            messages: [MessageSummary::empty(); MAX_MESSAGES],
            message_count: 0,
            selected_guild: 0,
            selected_channel: 0,
            token_configured: false,
            heartbeat_count: 0,
        }
    }
}

#[derive(Clone, Copy)]
pub enum DiscordUiAction {
    SelectGuild(usize),
    SelectChannel(usize),
    RefreshConfig,
    SendComposedMessage,
}

#[derive(Clone, Copy)]
pub struct DiscordDiag {
    pub state: DiscordState,
    pub config_present: bool,
    pub token_present: bool,
    pub token_len: usize,
    pub guild_count: usize,
    pub channel_count: usize,
    pub message_count: usize,
    pub heartbeat_count: u32,
    pub reconnect_attempts: u32,
    pub transport_connected: bool,
    pub transport_handle_id: u32,
    pub bridge_ip: net::Ipv4Addr,
    pub bridge_port: u16,
    pub poll_interval_ticks: u32,
    pub last_sync_tick: u32,
    pub last_message_id: [u8; ID_MAX_LEN],
    pub last_message_id_len: usize,
    pub last_error: [u8; ERROR_MAX_LEN],
    pub last_error_len: usize,
}

impl DiscordDiag {
    pub fn last_error_str(&self) -> &str {
        str::from_utf8(&self.last_error[..self.last_error_len]).unwrap_or("")
    }

    pub fn last_message_id_str(&self) -> &str {
        str::from_utf8(&self.last_message_id[..self.last_message_id_len]).unwrap_or("")
    }
}

pub struct DiscordClient {
    config: DiscordConfig,
    config_present: bool,
    state: DiscordState,
    guilds: [GuildSummary; MAX_GUILDS],
    guild_count: usize,
    channels: [ChannelSummary; MAX_CHANNELS],
    channel_count: usize,
    messages: [MessageSummary; MAX_MESSAGES],
    message_count: usize,
    message_head: usize,
    selected_guild: usize,
    selected_channel: usize,
    heartbeat_count: u32,
    heartbeat_interval_ticks: u32,
    last_heartbeat_tick: u32,
    reconnect_attempts: u32,
    transport_connected: bool,
    transport_handle_id: u32,
    next_bridge_attempt_tick: u32,
    pending_request_kind: BridgeRequestKind,
    pending_request_port: u16,
    pending_request_deadline: u32,
    outbox: [u8; MESSAGE_MAX_LEN],
    outbox_len: usize,
    outbox_active: bool,
    last_tick_processed: u32,
    last_sync_tick: u32,
    last_poll_tick: u32,
    last_message_id: [u8; ID_MAX_LEN],
    last_message_id_len: usize,
    last_error: [u8; ERROR_MAX_LEN],
    last_error_len: usize,
}

impl DiscordClient {
    pub fn from_fs() -> Self {
        match load_config_from_fs() {
            Ok(config) => Self::new(config),
            Err(error) => {
                let mut client = Self::new(DiscordConfig::empty());
                client.config_present = !matches!(error, DiscordError::MissingConfig);
                client.state = match error {
                    DiscordError::MissingConfig => DiscordState::MissingConfig,
                    DiscordError::MissingToken => DiscordState::MissingToken,
                    _ => DiscordState::Error,
                };
                client.set_error(error.as_str());
                client
            }
        }
    }

    pub fn new(config: DiscordConfig) -> Self {
        let mut client = Self {
            config,
            config_present: true,
            state: DiscordState::Ready,
            guilds: [GuildSummary::empty(); MAX_GUILDS],
            guild_count: 0,
            channels: [ChannelSummary::empty(); MAX_CHANNELS],
            channel_count: 0,
            messages: [MessageSummary::empty(); MAX_MESSAGES],
            message_count: 0,
            message_head: 0,
            selected_guild: 0,
            selected_channel: 0,
            heartbeat_count: 0,
            heartbeat_interval_ticks: HEARTBEAT_TICKS,
            last_heartbeat_tick: 0,
            reconnect_attempts: 0,
            transport_connected: false,
            transport_handle_id: 0,
            next_bridge_attempt_tick: 0,
            pending_request_kind: BridgeRequestKind::None,
            pending_request_port: 0,
            pending_request_deadline: 0,
            outbox: [0; MESSAGE_MAX_LEN],
            outbox_len: 0,
            outbox_active: false,
            last_tick_processed: u32::MAX,
            last_sync_tick: 0,
            last_poll_tick: 0,
            last_message_id: [0; ID_MAX_LEN],
            last_message_id_len: 0,
            last_error: [0; ERROR_MAX_LEN],
            last_error_len: 0,
        };

        if client.config.token_len == 0 {
            client.state = DiscordState::MissingToken;
            client.set_error(DiscordError::MissingToken.as_str());
            return client;
        }

        client.seed_catalogs();
        client.push_message("system", b"discord bridge mode ready", false);
        client.push_message("system", b"bot token loaded", false);
        client
    }

    pub fn reload_from_fs(&mut self) {
        match load_config_from_fs() {
            Ok(config) => {
                *self = Self::new(config);
            }
            Err(error) => {
                self.config = DiscordConfig::empty();
                self.config_present = !matches!(error, DiscordError::MissingConfig);
                self.state = match error {
                    DiscordError::MissingConfig => DiscordState::MissingConfig,
                    DiscordError::MissingToken => DiscordState::MissingToken,
                    _ => DiscordState::Error,
                };
                self.guild_count = 0;
                self.channel_count = 0;
                self.message_count = 0;
                self.message_head = 0;
                self.heartbeat_count = 0;
                self.last_heartbeat_tick = 0;
                self.reconnect_attempts = 0;
                self.transport_connected = false;
                self.transport_handle_id = 0;
                self.next_bridge_attempt_tick = 0;
                self.pending_request_kind = BridgeRequestKind::None;
                self.pending_request_port = 0;
                self.pending_request_deadline = 0;
                self.outbox_len = 0;
                self.outbox_active = false;
                self.last_tick_processed = u32::MAX;
                self.last_sync_tick = 0;
                self.last_poll_tick = 0;
                self.last_message_id_len = 0;
                self.set_error(error.as_str());
            }
        }
    }

    pub fn tick(&mut self, now_ticks: u32) {
        if self.state != DiscordState::Ready {
            return;
        }
        if self.last_tick_processed == now_ticks {
            return;
        }
        self.last_tick_processed = now_ticks;

        if self.last_heartbeat_tick == 0 {
            self.last_heartbeat_tick = now_ticks;
        }

        if now_ticks.wrapping_sub(self.last_heartbeat_tick) >= self.heartbeat_interval_ticks {
            self.last_heartbeat_tick = now_ticks;
            self.heartbeat_count = self.heartbeat_count.wrapping_add(1);
            if (self.heartbeat_count % 60) == 0 {
                self.push_message("system", b"bridge heartbeat", false);
            }
        }

        net::poll(now_ticks);
        self.poll_pending_request(now_ticks);

        if self.pending_request_kind != BridgeRequestKind::None {
            return;
        }
        if !tick_reached(now_ticks, self.next_bridge_attempt_tick) {
            return;
        }

        if self.outbox_active {
            let _ = self.start_send_request(now_ticks);
            return;
        }

        if now_ticks.wrapping_sub(self.last_poll_tick) >= self.config.poll_interval_ticks {
            self.last_poll_tick = now_ticks;
            let _ = self.start_sync_request(now_ticks);
        }
    }

    pub fn sync_now(&mut self) -> Result<(), DiscordError> {
        let now = timer::ticks();
        self.last_poll_tick = now;
        if self.pending_request_kind == BridgeRequestKind::None
            && tick_reached(now, self.next_bridge_attempt_tick)
        {
            self.start_sync_request(now)?;
        }
        Ok(())
    }

    pub fn on_ui_action(&mut self, action: DiscordUiAction) {
        match action {
            DiscordUiAction::SelectGuild(index) => {
                if index < self.guild_count {
                    self.selected_guild = index;
                    self.last_poll_tick = 0;
                }
            }
            DiscordUiAction::SelectChannel(index) => {
                if index < self.channel_count {
                    self.selected_channel = index;
                    self.last_poll_tick = 0;
                }
            }
            DiscordUiAction::RefreshConfig => self.reload_from_fs(),
            DiscordUiAction::SendComposedMessage => {}
        }
    }

    pub fn send_text_message(&mut self, text: &[u8]) -> Result<(), DiscordError> {
        if self.state != DiscordState::Ready {
            return Err(DiscordError::NotReady);
        }

        if text.is_empty() {
            return Ok(());
        }

        if text.len() > MESSAGE_MAX_LEN {
            return Err(DiscordError::MessageTooLong);
        }

        if self.outbox_active || self.pending_request_kind == BridgeRequestKind::Send {
            return Err(DiscordError::Busy);
        }

        self.outbox_len = copy_ascii(self.outbox.as_mut_slice(), text);
        self.outbox_active = self.outbox_len > 0;
        self.push_message("bot", text, true);
        self.clear_error();
        self.next_bridge_attempt_tick = timer::ticks();
        self.last_poll_tick = 0;
        Ok(())
    }

    pub fn state(&self) -> DiscordState {
        self.state
    }

    pub fn state_text(&self) -> &'static str {
        self.state.as_str()
    }

    pub fn config(&self) -> Option<DiscordConfig> {
        if self.config_present {
            Some(self.config)
        } else {
            None
        }
    }

    pub fn write_ui_snapshot(&self, out: &mut DiscordUiSnapshot) {
        out.state = self.state;
        out.guild_count = self.guild_count;
        for index in 0..self.guild_count {
            out.guilds[index] = self.guilds[index];
        }

        out.channel_count = self.channel_count;
        for index in 0..self.channel_count {
            out.channels[index] = self.channels[index];
        }

        out.message_count = self.message_count;
        if self.message_count > 0 {
            let start = if self.message_count < self.messages.len() {
                0
            } else {
                self.message_head
            };
            for slot in 0..self.message_count {
                let index = (start + slot) % self.messages.len();
                out.messages[slot] = self.messages[index];
            }
        }

        out.selected_guild = self.selected_guild;
        out.selected_channel = self.selected_channel;
        out.token_configured = self.config.token_len > 0;
        out.heartbeat_count = self.heartbeat_count;
    }

    pub fn diag(&self) -> DiscordDiag {
        DiscordDiag {
            state: self.state,
            config_present: self.config_present,
            token_present: self.config.token_len > 0,
            token_len: self.config.token_len,
            guild_count: self.guild_count,
            channel_count: self.channel_count,
            message_count: self.message_count,
            heartbeat_count: self.heartbeat_count,
            reconnect_attempts: self.reconnect_attempts,
            transport_connected: self.transport_connected,
            transport_handle_id: self.transport_handle_id,
            bridge_ip: self.config.bridge_ip,
            bridge_port: self.config.bridge_port,
            poll_interval_ticks: self.config.poll_interval_ticks,
            last_sync_tick: self.last_sync_tick,
            last_message_id: self.last_message_id,
            last_message_id_len: self.last_message_id_len,
            last_error: self.last_error,
            last_error_len: self.last_error_len,
        }
    }

    fn seed_catalogs(&mut self) {
        self.guild_count = 0;
        self.channel_count = 0;
        self.selected_guild = 0;
        self.selected_channel = 0;

        if self.guild_count < self.guilds.len() {
            let id = if self.config.default_guild_id_len > 0 {
                self.config.default_guild_id()
            } else {
                "guild"
            };
            self.guilds[self.guild_count].set(id, "configured");
            self.guild_count += 1;
        }

        if self.channel_count < self.channels.len() {
            let id = if self.config.default_channel_id_len > 0 {
                self.config.default_channel_id()
            } else {
                "channel"
            };
            self.channels[self.channel_count].set(id, "general");
            self.channel_count += 1;
        }
    }

    fn push_message(&mut self, author: &str, content: &[u8], local_echo: bool) {
        let slot = self.message_head;
        self.messages[slot].set(author, content, local_echo);
        self.message_head = (self.message_head + 1) % self.messages.len();
        if self.message_count < self.messages.len() {
            self.message_count += 1;
        }
    }

    fn push_message_bytes(&mut self, author: &[u8], content: &[u8], local_echo: bool) {
        let slot = self.message_head;
        self.messages[slot].set_bytes(author, content, local_echo);
        self.message_head = (self.message_head + 1) % self.messages.len();
        if self.message_count < self.messages.len() {
            self.message_count += 1;
        }
    }

    fn set_error(&mut self, text: &str) {
        self.last_error_len = copy_ascii(self.last_error.as_mut_slice(), text.as_bytes());
    }

    fn set_error_bytes(&mut self, text: &[u8]) {
        self.last_error_len = copy_ascii(self.last_error.as_mut_slice(), text);
    }

    fn clear_error(&mut self) {
        self.last_error.fill(0);
        self.last_error_len = 0;
    }

    fn selected_guild_id(&self) -> &[u8] {
        if self.selected_guild < self.guild_count {
            return &self.guilds[self.selected_guild].id[..self.guilds[self.selected_guild].id_len];
        }
        if self.config.default_guild_id_len > 0 {
            return &self.config.default_guild_id[..self.config.default_guild_id_len];
        }
        &[]
    }

    fn selected_channel_id(&self) -> &[u8] {
        if self.selected_channel < self.channel_count {
            return &self.channels[self.selected_channel].id
                [..self.channels[self.selected_channel].id_len];
        }
        if self.config.default_channel_id_len > 0 {
            return &self.config.default_channel_id[..self.config.default_channel_id_len];
        }
        &[]
    }

    fn last_message_numeric(&self) -> u64 {
        parse_u64_ascii(&self.last_message_id[..self.last_message_id_len]).unwrap_or(0)
    }

    fn start_sync_request(&mut self, now_ticks: u32) -> Result<(), DiscordError> {
        if self.state != DiscordState::Ready {
            return Err(DiscordError::NotReady);
        }
        if self.pending_request_kind != BridgeRequestKind::None {
            return Err(DiscordError::Busy);
        }
        if !net::is_online() {
            self.record_request_failure(BridgeRequestKind::Sync, DiscordError::Net(net::NetError::NotInitialized), now_ticks);
            return Err(DiscordError::Net(net::NetError::NotInitialized));
        }

        let mut request = [0u8; BRIDGE_REQ_MAX_BYTES];
        let request_len = self.build_sync_request(&mut request)?;

        match net::udp_request(
            self.config.bridge_ip,
            self.config.bridge_port,
            &request[..request_len],
        ) {
            Ok(port) => {
                self.pending_request_kind = BridgeRequestKind::Sync;
                self.pending_request_port = port;
                self.pending_request_deadline = now_ticks.wrapping_add(BRIDGE_TIMEOUT_TICKS);
                Ok(())
            }
            Err(error) => {
                let wrapped = DiscordError::Net(error);
                self.record_request_failure(BridgeRequestKind::Sync, wrapped, now_ticks);
                Err(wrapped)
            }
        }
    }

    fn start_send_request(&mut self, now_ticks: u32) -> Result<(), DiscordError> {
        if self.state != DiscordState::Ready {
            return Err(DiscordError::NotReady);
        }
        if !self.outbox_active || self.outbox_len == 0 {
            return Err(DiscordError::InvalidConfig);
        }
        if self.pending_request_kind != BridgeRequestKind::None {
            return Err(DiscordError::Busy);
        }
        if !net::is_online() {
            self.record_request_failure(BridgeRequestKind::Send, DiscordError::Net(net::NetError::NotInitialized), now_ticks);
            return Err(DiscordError::Net(net::NetError::NotInitialized));
        }

        let channel_id = self.selected_channel_id();
        if channel_id.is_empty() {
            self.outbox_active = false;
            self.outbox_len = 0;
            return Err(DiscordError::NotReady);
        }

        let mut request = [0u8; BRIDGE_REQ_MAX_BYTES];
        let mut len = 0usize;
        push_bytes(&mut request, &mut len, b"SEND\t").ok_or(DiscordError::InvalidConfig)?;
        push_bytes(&mut request, &mut len, &self.config.token[..self.config.token_len])
            .ok_or(DiscordError::InvalidConfig)?;
        push_bytes(&mut request, &mut len, b"\t").ok_or(DiscordError::InvalidConfig)?;
        push_bytes(&mut request, &mut len, channel_id).ok_or(DiscordError::InvalidConfig)?;
        push_bytes(&mut request, &mut len, b"\t").ok_or(DiscordError::InvalidConfig)?;
        push_sanitized_message(
            &mut request,
            &mut len,
            &self.outbox[..self.outbox_len],
        )
        .ok_or(DiscordError::MessageTooLong)?;
        push_bytes(&mut request, &mut len, b"\n").ok_or(DiscordError::InvalidConfig)?;

        match net::udp_request(self.config.bridge_ip, self.config.bridge_port, &request[..len]) {
            Ok(port) => {
                self.pending_request_kind = BridgeRequestKind::Send;
                self.pending_request_port = port;
                self.pending_request_deadline = now_ticks.wrapping_add(BRIDGE_TIMEOUT_TICKS);
                Ok(())
            }
            Err(error) => {
                let wrapped = DiscordError::Net(error);
                self.record_request_failure(BridgeRequestKind::Send, wrapped, now_ticks);
                Err(wrapped)
            }
        }
    }

    fn poll_pending_request(&mut self, now_ticks: u32) {
        if self.pending_request_kind == BridgeRequestKind::None || self.pending_request_port == 0 {
            return;
        }

        let kind = self.pending_request_kind;
        let port = self.pending_request_port;

        let mut response = [0u8; BRIDGE_RESP_MAX_BYTES];
        match net::udp_request_poll(port, &mut response) {
            Ok(Some(received)) => {
                self.finish_pending_request();
                let result = match kind {
                    BridgeRequestKind::Sync => self.apply_sync_response(&response[..received]),
                    BridgeRequestKind::Send => self.apply_send_response(&response[..received]),
                    BridgeRequestKind::None => Ok(()),
                };

                match result {
                    Ok(()) => {
                        self.on_bridge_connected();
                        if kind == BridgeRequestKind::Send {
                            self.outbox_active = false;
                            self.outbox_len = 0;
                            self.last_poll_tick = 0;
                        }
                    }
                    Err(error) => {
                        self.record_request_failure(kind, error, now_ticks);
                    }
                }
            }
            Ok(None) => {
                if tick_reached(now_ticks, self.pending_request_deadline) {
                    let _ = net::udp_request_cancel(port);
                    self.finish_pending_request();
                    self.record_request_failure(
                        kind,
                        DiscordError::Net(net::NetError::Timeout),
                        now_ticks,
                    );
                }
            }
            Err(error) => {
                self.finish_pending_request();
                self.record_request_failure(kind, DiscordError::Net(error), now_ticks);
            }
        }
    }

    fn finish_pending_request(&mut self) {
        self.pending_request_kind = BridgeRequestKind::None;
        self.pending_request_port = 0;
        self.pending_request_deadline = 0;
    }

    fn on_bridge_connected(&mut self) {
        if !self.transport_connected {
            self.transport_handle_id = self.transport_handle_id.wrapping_add(1).max(1);
            self.push_message("system", b"bridge connected", false);
        }
        self.transport_connected = true;
        self.last_sync_tick = timer::ticks();
        self.clear_error();
        self.next_bridge_attempt_tick = 0;
    }

    fn record_request_failure(
        &mut self,
        kind: BridgeRequestKind,
        error: DiscordError,
        now_ticks: u32,
    ) {
        self.transport_connected = false;
        self.reconnect_attempts = self.reconnect_attempts.wrapping_add(1);
        self.set_error(error.as_str());
        self.next_bridge_attempt_tick = now_ticks.wrapping_add(BRIDGE_RETRY_TICKS);
        if kind == BridgeRequestKind::Send {
            self.outbox_active = false;
            self.outbox_len = 0;
        }
    }

    fn build_sync_request(&self, out: &mut [u8]) -> Result<usize, DiscordError> {
        let mut len = 0usize;
        push_bytes(out, &mut len, b"SYNC\t").ok_or(DiscordError::InvalidConfig)?;
        push_bytes(out, &mut len, &self.config.token[..self.config.token_len])
            .ok_or(DiscordError::InvalidConfig)?;
        push_bytes(out, &mut len, b"\t").ok_or(DiscordError::InvalidConfig)?;
        push_bytes(out, &mut len, self.selected_guild_id()).ok_or(DiscordError::InvalidConfig)?;
        push_bytes(out, &mut len, b"\t").ok_or(DiscordError::InvalidConfig)?;
        push_bytes(out, &mut len, self.selected_channel_id()).ok_or(DiscordError::InvalidConfig)?;
        push_bytes(out, &mut len, b"\t").ok_or(DiscordError::InvalidConfig)?;
        push_bytes(out, &mut len, &self.last_message_id[..self.last_message_id_len])
            .ok_or(DiscordError::InvalidConfig)?;
        push_bytes(out, &mut len, b"\n").ok_or(DiscordError::InvalidConfig)?;
        Ok(len)
    }

    fn apply_sync_response(&mut self, data: &[u8]) -> Result<(), DiscordError> {
        if data.is_empty() {
            return Err(DiscordError::InvalidResponse);
        }

        let previous_max = self.last_message_numeric();
        let mut max_message = previous_max;
        let mut max_message_id = self.last_message_id;
        let mut max_message_id_len = self.last_message_id_len;

        let mut parsed_guilds = [GuildSummary::empty(); MAX_GUILDS];
        let mut parsed_channels = [ChannelSummary::empty(); MAX_CHANNELS];
        let mut guild_count = 0usize;
        let mut channel_count = 0usize;

        let mut saw_ok = false;

        let mut offset = 0usize;
        while offset < data.len() {
            let line_start = offset;
            while offset < data.len() && data[offset] != b'\n' {
                offset += 1;
            }
            let mut line = &data[line_start..offset];
            if offset < data.len() {
                offset += 1;
            }
            if line.ends_with(&[b'\r']) {
                line = &line[..line.len().saturating_sub(1)];
            }
            if line.is_empty() {
                continue;
            }

            let mut fields = [&[][..]; 5];
            let field_count = split_tabs(line, &mut fields);
            if field_count == 0 {
                continue;
            }

            if fields[0] == b"OK" {
                saw_ok = true;
                continue;
            }
            if fields[0] == b"E" {
                break;
            }
            if fields[0] == b"ERR" {
                if field_count > 1 {
                    self.set_error_bytes(fields[1]);
                }
                return Err(DiscordError::BridgeRejected);
            }

            if fields[0] == b"G" {
                if field_count >= 3 && guild_count < parsed_guilds.len() {
                    parsed_guilds[guild_count].set_bytes(fields[1], fields[2]);
                    guild_count += 1;
                }
                continue;
            }

            if fields[0] == b"C" {
                if field_count >= 3 && channel_count < parsed_channels.len() {
                    parsed_channels[channel_count].set_bytes(fields[1], fields[2]);
                    channel_count += 1;
                }
                continue;
            }

            if fields[0] == b"M" {
                if field_count >= 4 {
                    let message_id = parse_u64_ascii(fields[1]).unwrap_or(0);
                    if message_id > max_message {
                        max_message = message_id;
                        max_message_id_len = copy_ascii(&mut max_message_id, fields[1]);
                        self.push_message_bytes(fields[2], fields[3], false);
                    }
                }
                continue;
            }
        }

        if !saw_ok {
            return Err(DiscordError::InvalidResponse);
        }

        if guild_count > 0 {
            self.guilds = [GuildSummary::empty(); MAX_GUILDS];
            for index in 0..guild_count {
                self.guilds[index] = parsed_guilds[index];
            }
            self.guild_count = guild_count;
            if self.selected_guild >= self.guild_count {
                self.selected_guild = self.guild_count.saturating_sub(1);
            }
        }

        if channel_count > 0 {
            self.channels = [ChannelSummary::empty(); MAX_CHANNELS];
            for index in 0..channel_count {
                self.channels[index] = parsed_channels[index];
            }
            self.channel_count = channel_count;
            if self.selected_channel >= self.channel_count {
                self.selected_channel = self.channel_count.saturating_sub(1);
            }
        }

        if max_message > previous_max {
            self.last_message_id = max_message_id;
            self.last_message_id_len = max_message_id_len;
        }

        Ok(())
    }

    fn apply_send_response(&mut self, response: &[u8]) -> Result<(), DiscordError> {
        if response.is_empty() {
            return Err(DiscordError::InvalidResponse);
        }
        let first_line_end = response
            .iter()
            .position(|byte| *byte == b'\n')
            .unwrap_or(response.len());
        let mut first_line = &response[..first_line_end];
        if first_line.ends_with(&[b'\r']) {
            first_line = &first_line[..first_line.len().saturating_sub(1)];
        }

        if first_line.starts_with(b"SENT") || first_line == b"OK" {
            return Ok(());
        }

        if first_line.starts_with(b"ERR\t") {
            self.set_error_bytes(&first_line[4..]);
            return Err(DiscordError::BridgeRejected);
        }

        Err(DiscordError::InvalidResponse)
    }
}

pub fn load_config_from_fs() -> Result<DiscordConfig, DiscordError> {
    let mut buffer = [0u8; CONFIG_MAX_BYTES];
    let read = match fs::read_file(CONFIG_FILE, &mut buffer) {
        Ok(read) => read,
        Err(fs::FsError::NotFound) => return Err(DiscordError::MissingConfig),
        Err(error) => return Err(DiscordError::Fs(error)),
    };

    parse_config(&buffer[..read.copied_size])
}

fn parse_config(data: &[u8]) -> Result<DiscordConfig, DiscordError> {
    let text = str::from_utf8(data).map_err(|_| DiscordError::InvalidConfig)?;
    let mut config = DiscordConfig::empty();

    for raw_line in text.lines() {
        let line = raw_line.trim();
        if line.is_empty() || line.starts_with('#') || line.starts_with(';') {
            continue;
        }

        // Support both line-based config and single-line "key=value key=value".
        for token in line.split_whitespace() {
            if token.starts_with('#') || token.starts_with(';') {
                break;
            }
            let Some((key, value)) = token.split_once('=') else {
                continue;
            };
            apply_config_key_value(&mut config, key.trim(), value.trim())?;
        }
    }

    if config.token_len == 0 {
        return Err(DiscordError::MissingToken);
    }

    Ok(config)
}

fn apply_config_key_value(
    config: &mut DiscordConfig,
    key: &str,
    value: &str,
) -> Result<(), DiscordError> {
    match key {
        "bot_token" => {
            if value.len() > config.token.len() {
                return Err(DiscordError::TokenTooLong);
            }
            config.token_len = copy_token(config.token.as_mut_slice(), value.as_bytes());
        }
        "default_guild_id" => {
            config.default_guild_id_len =
                copy_ascii(config.default_guild_id.as_mut_slice(), value.as_bytes());
        }
        "default_channel_id" => {
            config.default_channel_id_len =
                copy_ascii(config.default_channel_id.as_mut_slice(), value.as_bytes());
        }
        "bridge_ip" => {
            if value.is_empty() {
                return Ok(());
            }
            let Some(parsed) = net::parse_ipv4_literal(value) else {
                return Err(DiscordError::InvalidConfig);
            };
            config.bridge_ip = parsed;
        }
        "bridge_port" => {
            let Some(parsed) = parse_u16_ascii(value.as_bytes()) else {
                return Err(DiscordError::InvalidConfig);
            };
            if parsed == 0 {
                return Err(DiscordError::InvalidConfig);
            }
            config.bridge_port = parsed;
        }
        "poll_ticks" => {
            let Some(parsed) = parse_u32_ascii(value.as_bytes()) else {
                return Err(DiscordError::InvalidConfig);
            };
            config.poll_interval_ticks = parsed.clamp(MIN_POLL_TICKS, MAX_POLL_TICKS);
        }
        _ => {}
    }

    Ok(())
}

fn split_tabs<'a>(line: &'a [u8], out: &mut [&'a [u8]]) -> usize {
    if out.is_empty() {
        return 0;
    }

    let mut count = 0usize;
    let mut start = 0usize;
    let mut index = 0usize;

    while index <= line.len() {
        if index == line.len() || line[index] == b'\t' {
            if count < out.len() {
                out[count] = &line[start..index];
                count += 1;
            }
            start = index.saturating_add(1);
            if count >= out.len() && index < line.len() {
                break;
            }
        }
        index += 1;
    }

    count
}

fn parse_u16_ascii(bytes: &[u8]) -> Option<u16> {
    if bytes.is_empty() {
        return None;
    }

    let mut value = 0u32;
    for byte in bytes.iter().copied() {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?;
        value = value.checked_add((byte - b'0') as u32)?;
        if value > u16::MAX as u32 {
            return None;
        }
    }

    Some(value as u16)
}

fn parse_u32_ascii(bytes: &[u8]) -> Option<u32> {
    if bytes.is_empty() {
        return None;
    }

    let mut value = 0u64;
    for byte in bytes.iter().copied() {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?;
        value = value.checked_add((byte - b'0') as u64)?;
        if value > u32::MAX as u64 {
            return None;
        }
    }

    Some(value as u32)
}

fn parse_u64_ascii(bytes: &[u8]) -> Option<u64> {
    if bytes.is_empty() {
        return None;
    }

    let mut value = 0u64;
    for byte in bytes.iter().copied() {
        if !byte.is_ascii_digit() {
            return None;
        }
        value = value.checked_mul(10)?;
        value = value.checked_add((byte - b'0') as u64)?;
    }

    Some(value)
}

fn tick_reached(now: u32, target: u32) -> bool {
    now.wrapping_sub(target) < (u32::MAX / 2)
}

fn push_bytes(out: &mut [u8], len: &mut usize, src: &[u8]) -> Option<()> {
    if *len + src.len() > out.len() {
        return None;
    }

    out[*len..*len + src.len()].copy_from_slice(src);
    *len += src.len();
    Some(())
}

fn push_sanitized_message(out: &mut [u8], len: &mut usize, src: &[u8]) -> Option<()> {
    for byte in src.iter().copied() {
        if *len >= out.len() {
            return None;
        }
        out[*len] = match byte {
            b'\n' | b'\r' | b'\t' => b' ',
            0x20..=0x7E => byte,
            _ => b'?',
        };
        *len += 1;
    }
    Some(())
}

fn copy_ascii(dst: &mut [u8], src: &[u8]) -> usize {
    let mut written = 0usize;
    for byte in src.iter().copied() {
        if written >= dst.len() {
            break;
        }
        dst[written] = sanitize_ascii(byte);
        written += 1;
    }
    written
}

fn copy_token(dst: &mut [u8], src: &[u8]) -> usize {
    let mut written = 0usize;
    for byte in src.iter().copied() {
        if written >= dst.len() {
            break;
        }
        if !(0x21..=0x7E).contains(&byte) {
            continue;
        }
        dst[written] = byte;
        written += 1;
    }
    written
}

fn sanitize_ascii(byte: u8) -> u8 {
    match byte {
        0x20..=0x7E => byte,
        _ => b'?',
    }
}
