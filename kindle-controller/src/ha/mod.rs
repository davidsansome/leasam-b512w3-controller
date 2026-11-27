//! Home Assistant client, on its own thread.
//!
//! It keeps one WebSocket connection open, reconnecting with backoff, and:
//! * subscribes to the entities the UI needs (`subscribe_entities`, so only those entities'
//!   changes are sent to us, not every state change in the house);
//! * makes service calls the UI asks for;
//! * registers the controller as a mobile_app device and pushes its diagnostic sensors.
//!
//! The UI hears about all of this through `HaEvent`s.

pub mod entities;
pub mod mobile_app;
pub mod resolve;
mod wake;

use std::collections::HashMap;
use std::io::ErrorKind;
use std::net::{TcpStream, ToSocketAddrs};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::time::{Duration, Instant};

use anyhow::{Context, Result, anyhow, bail};
use log::{debug, info, warn};
use serde_json::{Value, json};
use tungstenite::{HandshakeError, Message, WebSocket};

use crate::config::{DeviceConfig, HaConfig};
use crate::diagnostics;
use entities::{EntityMap, apply_compressed};
use mobile_app::StoredState;
use resolve::{AirconIds, Candidates, Registry};
use wake::{Sleeper, Waker};

const CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const PING_INTERVAL: Duration = Duration::from_secs(30);
/// How often the battery's charging status is checked, so a change reaches HA quickly.
const CHARGING_CHECK_INTERVAL: Duration = Duration::from_secs(60);
const PING_TIMEOUT: Duration = Duration::from_secs(20);
const MAX_BACKOFF_SECS: u64 = 60;
/// Registry changes often come in bursts (adding a device creates all its entities).
const REGISTRY_DEBOUNCE: Duration = Duration::from_secs(2);

/// Where HA is, parsed from `home_assistant.url`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpBase {
    /// `host:port`
    pub authority: String,
    /// Path HA is served under, without a trailing slash; usually empty.
    pub path_prefix: String,
}

impl HttpBase {
    pub fn parse(url: &str) -> Result<Self> {
        let url = url.trim();
        if url.starts_with("https://") || url.starts_with("wss://") {
            bail!("HTTPS isn't supported yet; use HA's plain http:// address on your LAN");
        }
        let rest = url
            .strip_prefix("http://")
            .or_else(|| url.strip_prefix("ws://"))
            .ok_or_else(|| anyhow!("home_assistant.url must start with http://"))?;
        let (authority, path) = rest.split_once('/').unwrap_or((rest, ""));
        if authority.is_empty() {
            bail!("no host in home_assistant.url");
        }
        let has_port = authority.rsplit_once(':').is_some_and(|(_, p)| p.chars().all(|c| c.is_ascii_digit()))
            && !authority.ends_with(']');
        let authority = if has_port { authority.to_string() } else { format!("{authority}:80") };
        let path = path.trim_matches('/');
        let path_prefix = if path.is_empty() { String::new() } else { format!("/{path}") };
        Ok(Self { authority, path_prefix })
    }

    pub fn websocket_url(&self) -> String {
        format!("ws://{}{}/api/websocket", self.authority, self.path_prefix)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct ServiceCall {
    pub domain: String,
    pub service: String,
    pub entity_id: String,
    pub data: Option<Value>,
}

impl ServiceCall {
    pub fn new(domain: &str, service: &str, entity_id: &str) -> Self {
        Self { domain: domain.into(), service: service.into(), entity_id: entity_id.into(), data: None }
    }
}

/// One hour of a weather forecast.
#[derive(Clone, Debug, PartialEq)]
pub struct ForecastHour {
    /// Unix seconds.
    pub time: f64,
    pub temperature: Option<f64>,
    /// mm in that hour.
    pub precipitation: Option<f64>,
    pub condition: String,
}

/// Recorded values of numeric entities: entity id → (Unix seconds, value), oldest first.
pub type History = HashMap<String, Vec<(f64, f64)>>;

#[derive(Debug)]
pub enum HaEvent {
    /// Authenticated and the first entity snapshot has arrived.
    Connected,
    Disconnected { reason: String, attempt: u32, retry_in: Duration },
    /// The entity ids to use for the aircon. Sent before the first `Entities` of each
    /// connection, and again whenever they change (e.g. HA renamed them).
    Aircon(AirconIds),
    /// The full current state of the followed entities.
    Entities(Arc<EntityMap>),
    ServiceCallFailed { call: ServiceCall, error: String },
    /// Hourly forecast for the configured weather entity.
    Forecast(Vec<ForecastHour>),
    /// Recent history for the room sensors, sent after each connect and whenever the aircon's
    /// room list changes.
    History(History),
}

enum Command {
    CallService(ServiceCall),
    /// The network has (re)appeared: if waiting to reconnect, try now.
    RetryNow,
}

/// Why a session ended without an error.
enum SessionEnd {
    UiGone,
}

pub struct HaHandle {
    tx: Sender<Command>,
    /// Wakes the client thread, which otherwise sleeps until HA sends something.
    waker: Waker,
}

impl HaHandle {
    pub fn call_service(&self, call: ServiceCall) {
        let _ = self.tx.send(Command::CallService(call));
        self.waker.wake();
    }

    /// Skips the rest of the reconnect backoff, e.g. because Wi-Fi just connected.
    pub fn retry_now(&self) {
        let _ = self.tx.send(Command::RetryNow);
        self.waker.wake();
    }
}

pub struct Options {
    pub ha: HaConfig,
    pub device: DeviceConfig,
    pub state_path: PathBuf,
    /// Entities to keep up to date, besides the aircon's own.
    pub entities: Vec<String>,
    /// The aircon entity ids from the config.
    pub aircon: AirconIds,
    /// HA device the aircon's entities are on. If set, they're looked up in the registry,
    /// so renamed entities are still found.
    pub aircon_device: Option<String>,
    /// Weather entity to fetch an hourly forecast for.
    pub weather: Option<String>,
    /// How much history to fetch for the room sensors.
    pub history_window: Duration,
}

const FORECAST_INTERVAL: Duration = Duration::from_secs(30 * 60);

/// Starts the client thread. `on_event` is called from that thread.
pub fn spawn(opts: Options, on_event: impl Fn(HaEvent) + Send + 'static) -> Result<HaHandle> {
    let base = HttpBase::parse(&opts.ha.url)?;
    let (tx, rx) = mpsc::channel();
    let (waker, sleeper) = wake::pair()?;
    std::thread::Builder::new()
        .name("ha".into())
        .spawn(move || run_forever(&opts, &base, &rx, &sleeper, &on_event))?;
    Ok(HaHandle { tx, waker })
}

fn run_forever(
    opts: &Options,
    base: &HttpBase,
    rx: &Receiver<Command>,
    sleeper: &Sleeper,
    on_event: &impl Fn(HaEvent),
) {
    let mut failures: u32 = 0;
    loop {
        info!("connecting to {}", base.websocket_url());
        let mut connected = false;
        let result = connect(base, &opts.ha.token)
            .and_then(|ws| Session::new(opts, base, ws, rx, sleeper, on_event, &mut connected).run());
        let reason = match result {
            Ok(SessionEnd::UiGone) => return,
            Err(e) => format!("{e:#}"),
        };
        if connected {
            failures = 0;
        }
        failures += 1;
        let retry_in = Duration::from_secs((1u64 << (failures - 1).min(6)).min(MAX_BACKOFF_SECS));
        warn!("not connected to HA: {reason}; retrying in {}s", retry_in.as_secs());
        on_event(HaEvent::Disconnected { reason, attempt: failures, retry_in });

        let until = Instant::now() + retry_in;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            match rx.recv_timeout(left) {
                Ok(Command::CallService(call)) => {
                    on_event(HaEvent::ServiceCallFailed { call, error: "not connected".into() })
                }
                Ok(Command::RetryNow) => {
                    info!("network is back; reconnecting now");
                    failures = 0;
                    break;
                }
                Err(RecvTimeoutError::Timeout) => break,
                Err(RecvTimeoutError::Disconnected) => return,
            }
        }
    }
}

type Ws = WebSocket<TcpStream>;

fn connect(base: &HttpBase, token: &str) -> Result<Ws> {
    let addr = base
        .authority
        .to_socket_addrs()
        .with_context(|| format!("resolving {}", base.authority))?
        .next()
        .ok_or_else(|| anyhow!("can't resolve {}", base.authority))?;
    let stream = TcpStream::connect_timeout(&addr, CONNECT_TIMEOUT).with_context(|| format!("connecting to {addr}"))?;
    stream.set_nodelay(true)?;
    stream.set_read_timeout(Some(CONNECT_TIMEOUT))?;
    stream.set_write_timeout(Some(CONNECT_TIMEOUT))?;
    let (mut ws, _) = tungstenite::client(base.websocket_url(), stream).map_err(|e| match e {
        HandshakeError::Failure(e) => anyhow!("websocket handshake: {e}"),
        HandshakeError::Interrupted(_) => anyhow!("websocket handshake timed out"),
    })?;

    let hello = read_json(&mut ws)?;
    if hello["type"] != "auth_required" {
        bail!("unexpected first message: {hello}");
    }
    send_json(&mut ws, &json!({"type": "auth", "access_token": token}))?;
    let reply = read_json(&mut ws)?;
    match reply["type"].as_str() {
        Some("auth_ok") => {
            info!("authenticated (HA {})", reply["ha_version"].as_str().unwrap_or("?"));
        }
        Some("auth_invalid") => {
            bail!("authentication failed: {}", reply["message"].as_str().unwrap_or("invalid token"))
        }
        _ => bail!("unexpected auth reply: {reply}"),
    }
    // From here on the session waits in poll() rather than blocking in reads.
    ws.get_mut().set_read_timeout(None)?;
    ws.get_mut().set_nonblocking(true)?;
    Ok(ws)
}

/// Blocking read of the next JSON text message (only used while the socket is blocking).
fn read_json(ws: &mut Ws) -> Result<Value> {
    loop {
        match ws.read()? {
            Message::Text(t) => return Ok(serde_json::from_str(t.as_str())?),
            Message::Close(f) => bail!("connection closed ({f:?})"),
            _ => {}
        }
    }
}

fn send_json(ws: &mut Ws, v: &Value) -> Result<()> {
    ws.send(Message::text(v.to_string())).context("websocket send")
}

fn is_timeout(e: &tungstenite::Error) -> bool {
    matches!(e, tungstenite::Error::Io(io) if matches!(io.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut))
}

/// What a request id is waiting for.
enum Pending {
    Subscribe,
    Unsubscribe,
    SubscribeEvents,
    EntityRegistry,
    DeviceRegistry,
    Forecast,
    History,
    CallService(ServiceCall),
    RegisterSensor { generation: u32 },
    UpdateSensors,
    Ping,
}

struct Session<'a, F: Fn(HaEvent)> {
    opts: &'a Options,
    base: &'a HttpBase,
    ws: Ws,
    rx: &'a Receiver<Command>,
    sleeper: &'a Sleeper,
    on_event: &'a F,
    connected: &'a mut bool,
    /// Data is queued that the socket couldn't take yet.
    write_pending: bool,
    next_id: u64,
    pending: HashMap<u64, Pending>,
    entities: EntityMap,
    subscription: Option<u64>,
    /// Where the aircon's entities might be, and which we last told the UI to use.
    candidates: Candidates,
    chosen: Option<AirconIds>,
    /// The room sensors listed by the aircon's Rooms entity, which we also follow.
    room_sensors: Vec<String>,
    /// Registry replies being collected: (entities, devices). `Some(None)` means it failed.
    registry_reply: (Option<Option<Value>>, Option<Option<Value>>),
    registry_events: Vec<u64>,
    registry_refresh_at: Option<Instant>,
    stored: StoredState,
    /// Bumped each time we (re)register, so stale register_sensor results are ignored.
    registration_generation: u32,
    reregistered: bool,
    ping_sent: Option<Instant>,
    last_ping: Instant,
    last_diagnostics: Instant,
    /// When the charging status was last checked, and what was last sent to HA.
    last_charging_check: Instant,
    last_charging_status: Option<String>,
    last_forecast: Option<Instant>,
}

impl<'a, F: Fn(HaEvent)> Session<'a, F> {
    fn new(
        opts: &'a Options,
        base: &'a HttpBase,
        ws: Ws,
        rx: &'a Receiver<Command>,
        sleeper: &'a Sleeper,
        on_event: &'a F,
        connected: &'a mut bool,
    ) -> Self {
        Self {
            opts,
            base,
            ws,
            rx,
            sleeper,
            on_event,
            connected,
            write_pending: false,
            next_id: 1,
            pending: HashMap::new(),
            entities: EntityMap::new(),
            subscription: None,
            candidates: Candidates::new(&opts.aircon, None),
            chosen: None,
            room_sensors: Vec::new(),
            registry_reply: (None, None),
            registry_events: Vec::new(),
            registry_refresh_at: None,
            stored: StoredState::load(&opts.state_path),
            registration_generation: 0,
            reregistered: false,
            ping_sent: None,
            last_ping: Instant::now(),
            last_diagnostics: Instant::now(),
            last_charging_check: Instant::now(),
            last_charging_status: None,
            last_forecast: None,
        }
    }

    /// Runs until the connection fails (Err), or the UI goes away or pauses us (Ok).
    fn run(mut self) -> Result<SessionEnd> {
        self.setup_mobile_app()?;
        if self.opts.aircon_device.is_some() {
            // Subscribes once the registry has been read.
            self.watch_registry()?;
            self.fetch_registry()?;
        } else {
            self.resubscribe()?;
        }
        // Room history is fetched once the Rooms entity has said which sensors to follow.
        self.fetch_forecast()?;
        loop {
            // Handle everything that has arrived.
            loop {
                match self.ws.read() {
                    Ok(Message::Text(t)) => self.handle(t.as_str())?,
                    Ok(Message::Close(f)) => bail!("connection closed by HA ({f:?})"),
                    Ok(_) => {}
                    Err(e) if is_timeout(&e) => break,
                    Err(e) => return Err(e).context("websocket read"),
                }
            }
            loop {
                match self.rx.try_recv() {
                    Ok(Command::CallService(call)) => self.call_service(call)?,
                    Ok(Command::RetryNow) => {}
                    Err(TryRecvError::Empty) => break,
                    Err(TryRecvError::Disconnected) => return Ok(SessionEnd::UiGone),
                }
            }
            self.timers()?;
            // Sends anything queued, including replies to HA's own pings.
            match self.ws.flush() {
                Ok(()) => self.write_pending = false,
                Err(e) if is_timeout(&e) => self.write_pending = true,
                Err(e) => return Err(e).context("websocket flush"),
            }
            // Sleep until HA sends something, the UI wants something, or a timer is due.
            let timeout = self.next_deadline().saturating_duration_since(Instant::now());
            self.sleeper.wait(self.ws.get_ref(), self.write_pending, timeout + Duration::from_millis(1));
        }
    }

    /// When `timers` next has something to do.
    fn next_deadline(&self) -> Instant {
        let mut at = match self.ping_sent {
            Some(sent) => sent + PING_TIMEOUT,
            None => self.last_ping + PING_INTERVAL,
        };
        let interval = Duration::from_secs(self.opts.ha.diagnostics_interval_secs.max(10));
        at = at.min(self.last_diagnostics + interval);
        at = at.min(self.last_charging_check + CHARGING_CHECK_INTERVAL);
        if let Some(f) = self.last_forecast {
            at = at.min(f + FORECAST_INTERVAL);
        }
        if let Some(r) = self.registry_refresh_at {
            at = at.min(r);
        }
        at
    }

    fn watch_registry(&mut self) -> Result<()> {
        for event in ["entity_registry_updated", "device_registry_updated"] {
            let id = self.request(json!({"type": "subscribe_events", "event_type": event}), Pending::SubscribeEvents)?;
            self.registry_events.push(id);
        }
        Ok(())
    }

    fn fetch_registry(&mut self) -> Result<()> {
        self.registry_refresh_at = None;
        self.registry_reply = (None, None);
        self.request(json!({"type": "config/entity_registry/list"}), Pending::EntityRegistry)?;
        self.request(json!({"type": "config/device_registry/list"}), Pending::DeviceRegistry)?;
        Ok(())
    }

    /// Once both registry replies are in, works out the aircon candidates, and subscribes
    /// again if they changed.
    fn registry_received(&mut self) -> Result<()> {
        let (Some(entities), Some(devices)) = &self.registry_reply else { return Ok(()) };
        let registry = match (entities, devices) {
            (Some(e), Some(d)) => Some(Registry::parse(e, d)),
            _ => None,
        };
        self.registry_reply = (None, None);
        let device = self.opts.aircon_device.as_deref().unwrap_or_default();
        let candidates = Candidates::new(&self.opts.aircon, registry.as_ref().map(|r| (r, device)));
        if candidates != self.candidates || self.subscription.is_none() {
            info!("aircon candidates on device {device:?}: {candidates:?}");
            self.candidates = candidates;
            self.resubscribe()?;
        }
        Ok(())
    }

    /// (Re)subscribes to the other entities plus all the aircon candidates and room sensors.
    fn resubscribe(&mut self) -> Result<()> {
        if let Some(old) = self.subscription.take() {
            self.request(json!({"type": "unsubscribe_events", "subscription": old}), Pending::Unsubscribe)?;
        }
        self.entities.clear();
        let mut ids = self.opts.entities.clone();
        ids.extend(self.candidates.all());
        ids.extend(self.room_sensors.iter().cloned());
        ids.sort();
        ids.dedup();
        self.subscribe(ids)
    }

    fn request(&mut self, mut msg: Value, pending: Pending) -> Result<u64> {
        let id = self.next_id;
        self.next_id += 1;
        msg["id"] = json!(id);
        debug!("-> {msg}");
        // On a non-blocking socket a send that can't complete stays queued; flushed later.
        match self.ws.send(Message::text(msg.to_string())) {
            Ok(()) => {}
            Err(e) if is_timeout(&e) => self.write_pending = true,
            Err(e) => return Err(e).context("websocket send"),
        }
        self.pending.insert(id, pending);
        Ok(id)
    }

    fn emit(&self, event: HaEvent) {
        (self.on_event)(event);
    }

    fn subscribe(&mut self, ids: Vec<String>) -> Result<()> {
        if ids.is_empty() {
            // Subscribing with an empty list would mean "everything".
            info!("no entities to follow");
            self.emit(HaEvent::Entities(Arc::new(EntityMap::new())));
            self.mark_connected();
            return Ok(());
        }
        info!("following {} entities", ids.len());
        let id = self.request(json!({"type": "subscribe_entities", "entity_ids": ids}), Pending::Subscribe)?;
        self.subscription = Some(id);
        Ok(())
    }

    fn fetch_history(&mut self, ids: &[String]) -> Result<()> {
        if ids.is_empty() {
            return Ok(());
        }
        let start = crate::clock::now() - self.opts.history_window.as_secs_f64();
        self.request(
            json!({
                "type": "history/history_during_period",
                "start_time": crate::clock::iso_utc(start),
                "entity_ids": ids,
                "minimal_response": true,
                "no_attributes": true,
                "significant_changes_only": false,
            }),
            Pending::History,
        )?;
        Ok(())
    }

    fn fetch_forecast(&mut self) -> Result<()> {
        let Some(weather) = self.opts.weather.clone() else { return Ok(()) };
        self.last_forecast = Some(Instant::now());
        self.request(
            json!({
                "type": "call_service",
                "domain": "weather",
                "service": "get_forecasts",
                "target": {"entity_id": weather},
                "service_data": {"type": "hourly"},
                "return_response": true,
            }),
            Pending::Forecast,
        )?;
        Ok(())
    }

    fn mark_connected(&mut self) {
        if !*self.connected {
            *self.connected = true;
            self.emit(HaEvent::Connected);
        }
    }

    fn call_service(&mut self, call: ServiceCall) -> Result<()> {
        info!("calling {}.{} on {}", call.domain, call.service, call.entity_id);
        let mut msg = json!({
            "type": "call_service",
            "domain": call.domain,
            "service": call.service,
            "target": {"entity_id": call.entity_id},
        });
        if let Some(data) = &call.data {
            msg["service_data"] = data.clone();
        }
        self.request(msg, Pending::CallService(call))?;
        Ok(())
    }

    fn handle(&mut self, text: &str) -> Result<()> {
        let v: Value = serde_json::from_str(text).context("bad JSON from HA")?;
        let id = v["id"].as_u64();
        match v["type"].as_str() {
            Some("result") => {
                let Some(pending) = id.and_then(|id| self.pending.remove(&id)) else { return Ok(()) };
                let success = v["success"].as_bool() == Some(true);
                let error = v["error"]["message"].as_str().unwrap_or("unknown error").to_string();
                self.handle_result(pending, success, error, &v["result"])?;
            }
            Some("pong") => {
                if let Some(id) = id {
                    self.pending.remove(&id);
                }
                self.ping_sent = None;
            }
            Some("event") if id.is_some() && id == self.subscription => {
                apply_compressed(&mut self.entities, &v["event"]);
                let chosen = self.candidates.choose(&self.entities);
                if self.chosen.as_ref() != Some(&chosen) {
                    info!("aircon entities: {chosen:?}");
                    self.chosen = Some(chosen.clone());
                    self.emit(HaEvent::Aircon(chosen));
                }
                self.emit(HaEvent::Entities(Arc::new(self.entities.clone())));
                self.mark_connected();
                self.follow_rooms()?;
            }
            Some("event") if id.is_some_and(|id| self.registry_events.contains(&id)) => {
                if registry_change_matters(&v["event"]) && self.registry_refresh_at.is_none() {
                    debug!("registry changed: {}", v["event"]["data"]);
                    self.registry_refresh_at = Some(Instant::now() + REGISTRY_DEBOUNCE);
                }
            }
            _ => debug!("<- {text}"),
        }
        Ok(())
    }

    /// When the aircon's room list names different sensors, follows those instead and fetches
    /// their history. While the list is unavailable, keeps following the last one.
    fn follow_rooms(&mut self) -> Result<()> {
        let rooms_id = self.chosen.as_ref().and_then(|c| c.rooms.as_deref());
        let Some(rooms) = entities::rooms(&self.entities, rooms_id) else { return Ok(()) };
        let mut sensors: Vec<String> = rooms.into_iter().map(|r| r.sensor).collect();
        sensors.sort();
        sensors.dedup();
        if sensors != self.room_sensors {
            info!("room sensors: {sensors:?}");
            self.room_sensors = sensors.clone();
            self.fetch_history(&sensors)?;
            self.resubscribe()?;
        }
        Ok(())
    }

    fn handle_result(&mut self, pending: Pending, success: bool, error: String, result: &Value) -> Result<()> {
        match pending {
            Pending::Subscribe => {
                if !success {
                    bail!("subscribe_entities failed: {error}");
                }
            }
            Pending::Unsubscribe => {}
            Pending::SubscribeEvents => {
                if !success {
                    warn!("can't watch the registry for renamed entities: {error}");
                }
            }
            Pending::EntityRegistry | Pending::DeviceRegistry => {
                if !success {
                    warn!("reading the registry failed ({error}); using the configured aircon entity ids");
                }
                let value = success.then(|| result.clone());
                if matches!(pending, Pending::EntityRegistry) {
                    self.registry_reply.0 = Some(value);
                } else {
                    self.registry_reply.1 = Some(value);
                }
                self.registry_received()?;
            }
            Pending::Forecast => {
                if !success {
                    warn!("weather forecast failed: {error}");
                } else if let Some(weather) = &self.opts.weather {
                    let hours = parse_forecast(result, weather);
                    debug!("forecast: {} hours", hours.len());
                    self.emit(HaEvent::Forecast(hours));
                }
            }
            Pending::History => {
                if !success {
                    warn!("history request failed: {error}");
                } else {
                    self.emit(HaEvent::History(parse_history(result)));
                }
            }
            Pending::CallService(call) => {
                if !success {
                    warn!("{}.{} on {} failed: {error}", call.domain, call.service, call.entity_id);
                    self.emit(HaEvent::ServiceCallFailed { call, error });
                }
            }
            Pending::RegisterSensor { generation } => {
                if generation != self.registration_generation {
                    return Ok(());
                }
                if success && mobile_app::register_result_ok(result) {
                    debug!("sensor registered");
                } else if !self.reregistered {
                    warn!("HA no longer knows our mobile_app registration; registering again");
                    self.reregistered = true;
                    self.stored.webhook_id = None;
                    self.stored.save(&self.opts.state_path);
                    self.setup_mobile_app()?;
                } else {
                    warn!("sensor registration rejected: {result}");
                }
            }
            Pending::UpdateSensors => {
                if !success || !mobile_app::webhook_status_ok(result) {
                    warn!("sensor update failed: {error} {result}");
                }
            }
            Pending::Ping => {}
        }
        Ok(())
    }

    /// Makes sure we're registered as a device, and (re)registers our sensors. Failing to
    /// register isn't fatal: the UI works without it, and we try again next connection.
    fn setup_mobile_app(&mut self) -> Result<()> {
        if self.stored.webhook_id.is_none() {
            match mobile_app::register(self.base, &self.opts.ha.token, &self.opts.device, &self.stored.device_id) {
                Ok(id) => {
                    self.stored.webhook_id = Some(id);
                    self.stored.save(&self.opts.state_path);
                }
                Err(e) => {
                    warn!("{e:#}; diagnostics won't be reported this connection");
                    return Ok(());
                }
            }
        }
        self.registration_generation += 1;
        let generation = self.registration_generation;
        let d = diagnostics::collect();
        debug!("diagnostics: {d:?}");
        for payload in mobile_app::register_sensor_payloads(&d) {
            self.webhook(&payload, Pending::RegisterSensor { generation })?;
        }
        self.last_diagnostics = Instant::now();
        self.last_charging_status = d.charging_status;
        Ok(())
    }

    fn send_diagnostics(&mut self) -> Result<()> {
        self.last_diagnostics = Instant::now();
        let d = diagnostics::collect();
        debug!("diagnostics: {d:?}");
        self.last_charging_status = d.charging_status.clone();
        self.webhook(&mobile_app::update_sensors_payload(&d), Pending::UpdateSensors)
    }

    fn webhook(&mut self, payload: &Value, pending: Pending) -> Result<()> {
        let Some(webhook_id) = self.stored.webhook_id.clone() else { return Ok(()) };
        self.request(
            json!({
                "type": "webhook/handle",
                "webhook_id": webhook_id,
                "method": "POST",
                "body": payload.to_string(),
                "headers": {"Content-Type": "application/json"},
            }),
            pending,
        )?;
        Ok(())
    }

    fn timers(&mut self) -> Result<()> {
        if let Some(sent) = self.ping_sent {
            if sent.elapsed() > PING_TIMEOUT {
                bail!("HA stopped answering pings");
            }
        } else if self.last_ping.elapsed() >= PING_INTERVAL {
            self.request(json!({"type": "ping"}), Pending::Ping)?;
            self.ping_sent = Some(Instant::now());
            self.last_ping = Instant::now();
        }
        if self.last_forecast.is_some_and(|t| t.elapsed() >= FORECAST_INTERVAL) {
            self.fetch_forecast()?;
        }
        if self.registry_refresh_at.is_some_and(|t| Instant::now() >= t) {
            self.fetch_registry()?;
        }
        let interval = Duration::from_secs(self.opts.ha.diagnostics_interval_secs.max(10));
        // The charging status is checked once a minute, and a change is sent straight away
        // rather than left until the next scheduled update.
        let mut charging_changed = false;
        if self.last_charging_check.elapsed() >= CHARGING_CHECK_INTERVAL {
            self.last_charging_check = Instant::now();
            charging_changed = diagnostics::charging_status() != self.last_charging_status;
        }
        if self.last_diagnostics.elapsed() >= interval || charging_changed {
            self.send_diagnostics()?;
        }
        Ok(())
    }
}

/// Whether an `entity_registry_updated` / `device_registry_updated` event could change
/// which entities are the aircon's: anything added or removed, renamed, moved to another
/// device, or enabled/disabled. Other updates (icons, firmware versions...) can't.
fn registry_change_matters(event: &Value) -> bool {
    let data = &event["data"];
    if data["action"] != "update" || !data["old_entity_id"].is_null() {
        return true;
    }
    const KEYS: [&str; 6] = ["device_id", "disabled_by", "name", "name_by_user", "original_name", "unique_id"];
    data["changes"].as_object().is_some_and(|c| KEYS.iter().any(|k| c.contains_key(*k)))
}

/// Parses a `weather.get_forecasts` response: `{"response": {"weather.x": {"forecast": [...]}}}`.
fn parse_forecast(result: &Value, weather: &str) -> Vec<ForecastHour> {
    result["response"][weather]["forecast"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|h| {
            Some(ForecastHour {
                time: crate::clock::parse_iso(h["datetime"].as_str()?)?,
                temperature: h["temperature"].as_f64(),
                precipitation: h["precipitation"].as_f64(),
                condition: h["condition"].as_str().unwrap_or_default().to_string(),
            })
        })
        .collect()
}

/// Parses a minimal `history/history_during_period` response:
/// `{"sensor.x": [{"s": "21.4", "lu": 1731212345.1}, ...]}`. Non-numeric states are skipped.
fn parse_history(result: &Value) -> History {
    let mut out = History::new();
    for (id, rows) in result.as_object().into_iter().flatten() {
        let points: Vec<(f64, f64)> = rows
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| {
                let t = r["lu"].as_f64().or_else(|| r["lc"].as_f64())?;
                let v: f64 = r["s"].as_str()?.parse().ok()?;
                v.is_finite().then_some((t, v))
            })
            .collect();
        out.insert(id.clone(), points);
    }
    out
}

/// `--list-entities`: prints the entities that are useful in the config.
pub fn list_entities(cfg: &HaConfig) -> Result<()> {
    let base = HttpBase::parse(&cfg.url)?;
    let mut ws = connect(&base, &cfg.token)?;
    ws.get_mut().set_nonblocking(false)?;
    ws.get_mut().set_read_timeout(Some(Duration::from_secs(30)))?;
    send_json(&mut ws, &json!({"id": 1, "type": "get_states"}))?;
    let reply = loop {
        let v = read_json(&mut ws)?;
        if v["id"] == 1 {
            break v;
        }
    };
    let mut rows: Vec<(String, String, String)> = reply["result"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|s| {
            let id = s["entity_id"].as_str()?;
            let domain = id.split('.').next()?;
            let a = &s["attributes"];
            let wanted = match domain {
                "weather" | "climate" | "switch" | "select" | "number" | "input_number" | "input_boolean"
                | "input_select" => true,
                "sensor" | "binary_sensor" => {
                    a["device_class"] == "temperature" || id.contains("aircon") || id.contains("compressor")
                }
                _ => false,
            };
            if !wanted {
                return None;
            }
            let mut info = a["friendly_name"].as_str().unwrap_or("").to_string();
            if let Some(u) = a["unit_of_measurement"].as_str() {
                info += &format!(" [{u}]");
            }
            if let Some(opts) = a["options"].as_array() {
                let o: Vec<&str> = opts.iter().filter_map(Value::as_str).collect();
                info += &format!(" options={o:?}");
            }
            Some((id.to_string(), s["state"].as_str().unwrap_or("").to_string(), info))
        })
        .collect();
    rows.sort();
    for (id, state, info) in rows {
        println!("{id}	{state}	{info}");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_forecast_and_history() {
        let f = parse_forecast(
            &json!({"response": {"weather.home": {"forecast": [
                {"datetime": "2026-11-10T03:00:00+00:00", "temperature": 21.5, "precipitation": 0.4, "condition": "rainy"},
                {"datetime": "bad", "temperature": 1.0}
            ]}}}),
            "weather.home",
        );
        assert_eq!(f.len(), 1);
        assert_eq!(f[0].temperature, Some(21.5));
        assert_eq!(f[0].condition, "rainy");
        assert_eq!(f[0].time, crate::clock::parse_iso("2026-11-10T03:00:00Z").unwrap());

        let h = parse_history(&json!({"sensor.t": [{"s": "21.4", "lu": 100.0}, {"s": "unavailable", "lu": 200.0}, {"s": "21.9", "lu": 300.0}]}));
        assert_eq!(h["sensor.t"], vec![(100.0, 21.4), (300.0, 21.9)]);
    }

    #[test]
    fn parses_base_urls() {
        let b = HttpBase::parse("http://192.168.1.10:8123/").unwrap();
        assert_eq!(b, HttpBase { authority: "192.168.1.10:8123".into(), path_prefix: String::new() });
        assert_eq!(b.websocket_url(), "ws://192.168.1.10:8123/api/websocket");

        let b = HttpBase::parse("http://ha.lan/hass").unwrap();
        assert_eq!(b.authority, "ha.lan:80");
        assert_eq!(b.websocket_url(), "ws://ha.lan:80/hass/api/websocket");

        assert!(HttpBase::parse("https://ha.example.com").is_err());
        assert!(HttpBase::parse("ha.lan:8123").is_err());
    }
}
