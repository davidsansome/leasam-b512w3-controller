// ESP32 replacement for the Leasam B512W3 wall pad. Arduino-ESP32 core 3.x.
// Needs the PubSubClient library (arduino-cli lib install PubSubClient).
//
// One sketch for both boards. The board is picked from the chip being compiled for:
//   ESP32-C3      -> the PCB (../../kicad/esp32-board, README.md): VRES and tablet output sensing,
//                    tablet output switching and load shedding.
//   classic ESP32 -> the breadboard prototype (../../breadboard-prototype.md), on an
//                    ESP32-WROOM-32 DevKit. No tablet output.
//
// Sends the same frame the original pad sent: 9 x 15 us pulses that pull POWER low,
// 8 gaps (short 1.92 ms = 0, long 5.76 ms = 1), repeated every 450 ms.
// Frames are sent from power-up, whether or not Wi-Fi or MQTT are connected.
//
// Control is through Home Assistant, using MQTT discovery. The device "Aircon" appears
// under Settings > Devices once the MQTT integration is set up, with:
//   Power (switch), Mode (select: Cool / Heat / Fan only / Auto), Fan speed (select: Low / Medium / High),
//   Compressor (binary sensor, running), and diagnostics: AUX voltage with its minimum and maximum,
//   VRES and VRES minimum, Wi-Fi signal, Compressor lockout (seconds until the compressor may start),
//   Uptime, Last restart reason, Wi-Fi reconnects and Last Wi-Fi disconnect.
// On the PCB there is also a Tablet output switch, its status, voltage and shed/fault counts,
// and VRES at boot / Boot Wi-Fi wait.
// Power always starts off after a reboot or power cut. The last mode and fan speed are kept.
// The compressor is held off for 3 minutes after boot and after it last stopped.
//
// Thermostat (thermostat.h): Thermostat (switch: on = thermostat, off = continuous), Set point,
// Thermostat room (select: Average or one room), and config entities Thermostat hysteresis,
// Auto deadband and Minimum run time. Read-only: Operation (Off / Heating / Cooling / Fan: what
// the unit is really doing), Auto direction (Heat / Cool), Thermostat temperature, Thermostat
// status and Rooms (the room list, as attributes).
// In Cool or Heat mode with the thermostat on, the whole unit switches off ("idle") when the
// temperature reaches the set point, and back on once it's the hysteresis beyond it. Power,
// mode and fan keep their values. Once the compressor starts, it runs for at least the minimum
// run time. With no usable temperature the unit stays idle.
// Auto works like Heat or Cool around set point - deadband or set point + deadband. It starts
// on the side of the set point the temperature is on, and changes direction only once the
// temperature is past the other side's resume point. Auto needs the thermostat: with it off
// (continuous) the unit stays idle.
// Rooms come from Home Assistant: the "Aircon rooms" blueprint (../../home-assistant) publishes
// each room's name and sensor (retained, saved here too) and forwards its temperature every
// minute. A reading older than ROOM_STALE_MS, or any reading while HA is offline, isn't used.
// Thermostat settings and rooms are kept over a reboot.
// Firmware updates can be sent over Wi-Fi (ArduinoOTA, hostname below).
//
// Recovery: if Wi-Fi stays down for 1 minute the radio is restarted (frames keep going);
// after 5 minutes the board reboots. A loop watchdog reboots it if the main loop hangs.

#include <WiFi.h>
#include <ESPmDNS.h>
#include <ArduinoOTA.h>
#include <PubSubClient.h>
#include <Preferences.h>
#include <esp_system.h>
#include <esp_task_wdt.h>
#include "thermostat.h"

// ---------- user settings ----------
// Wi-Fi, MQTT broker and OTA password live in secrets.h (not committed).
// Copy secrets.example.h to secrets.h and fill it in.
#include "secrets.h"
const uint16_t MQTT_PORT = 1883;

// ---------- board ----------
#if defined(CONFIG_IDF_TARGET_ESP32C3)
#define BOARD_PCB 1
const char* HOSTNAME = "aircon";
const char* MODEL    = "Leasam B512W3 replacement";
constexpr int PIN_TX     = 4;          // -> R_G -> C_G -> Q_TX gate. High = pull POWER low.
constexpr int PIN_VRES   = 3;          // VSENSE: VRES via 150k / 10k
constexpr int PIN_AUX    = 0;          // AUX via 330k / 100k, 10 nF
constexpr int PIN_VTAB   = 1;          // tablet output via 150k / 10k
constexpr int PIN_TAB_EN = 5;          // high = tablet output on (via Q_EN)
constexpr int PIN_LED    = 10;         // on while MQTT is connected
constexpr float AUX_RATIO  = (330.0f + 100.0f) / 100.0f;   // not yet calibrated against a meter
constexpr float VRES_RATIO = (150.0f + 10.0f) / 10.0f;
constexpr float VTAB_RATIO = (150.0f + 10.0f) / 10.0f;
#elif defined(CONFIG_IDF_TARGET_ESP32)
#define BOARD_PCB 0
const char* HOSTNAME = "aircon-test";
const char* MODEL    = "Leasam B512W3 replacement (breadboard)";
constexpr int PIN_TX   = 25;           // -> C1 -> R1 -> Q1 base. High = pull POWER low.
constexpr int PIN_AUX  = 34;           // AUX via 100k / 33k (ADC1, input only)
constexpr int PIN_VRES = 35;           // VRES via R9 100k / R10 10k (ADC1, input only; stage 2)
constexpr int PIN_LED  = 2;            // on-board LED on most DevKits: on while MQTT is connected
// Calibrated against a meter on the AUX node (4.27 V on the meter, 4.33 V reported, same moment).
// Covers the 10k + 20k + 3k chain standing in for the 33k, and the ADC's own error.
constexpr float AUX_CAL    = 4.27f / 4.33f;
constexpr float AUX_RATIO  = (100.0f + 33.0f) / 33.0f * AUX_CAL;
constexpr float VRES_RATIO = (100.0f + 10.0f) / 10.0f;
#else
#error "Build for an ESP32-C3 (the PCB) or a classic ESP32 (the breadboard)"
#endif

// ---------- protocol timing ----------
constexpr uint16_t PULSE_US        = 15;
constexpr uint16_t SHORT_GAP_US    = 1920;
constexpr uint16_t LONG_GAP_US     = 5760;
constexpr uint32_t FRAME_PERIOD_MS = 450;

// ---------- frame bits (gap number 1..8 in transmission order) ----------
inline uint8_t bitMask(uint8_t gap) { return 1u << (gap - 1); }
const uint8_t BIT_FAN_HIGH = bitMask(1);
const uint8_t BIT_FAN_LOW  = bitMask(6);
const uint8_t BIT_FAN_MED  = bitMask(8);
const uint8_t COMP_BITS    = bitMask(4) | bitMask(5);
const uint8_t HEAT_BIT     = bitMask(7);

// ---------- protection ----------
constexpr uint32_t COMPRESSOR_MIN_OFF_MS = 3UL * 60UL * 1000UL;

// ---------- thermostat ----------
constexpr uint32_t ROOM_STALE_MS   = 5UL * 60UL * 1000UL;   // HA re-sends every minute
constexpr float SETPOINT_MIN = 16.0f, SETPOINT_MAX = 30.0f, SETPOINT_DEFAULT = 22.0f;
constexpr float HYST_MIN = 0.1f, HYST_MAX = 3.0f, HYST_DEFAULT = 0.5f;
constexpr float DEADBAND_MIN = 0.5f, DEADBAND_MAX = 5.0f, DEADBAND_DEFAULT = 1.0f;
constexpr int   MIN_RUN_MAX_MIN = 30, MIN_RUN_DEFAULT_MIN = 3;
constexpr size_t ROOM_NAME_MAX = 32, ROOM_SENSOR_MAX = 80;
constexpr uint32_t ROOMS_SETTLE_MS = 1000;   // room changes come in bursts

// ---------- tablet output (PCB) ----------
constexpr float    TAB_ON_MIN_V      = 14.0f;   // VRES must be at least this to switch on...
constexpr uint32_t TAB_ON_SETTLE_MS  = 10000;   // ...continuously for this long
constexpr float    TAB_SHED_V        = 13.0f;   // VRES below this -> switch off (protect the ESP32)
constexpr uint32_t TAB_SHED_MS       = 500;
constexpr float    TAB_FAULT_V       = 10.0f;   // output below this while on -> short/overload
constexpr uint32_t TAB_FAULT_MS      = 2000;
constexpr uint32_t TAB_RETRY_MS      = 60000;   // wait after a shed or fault
constexpr uint32_t TAB_BOOT_DELAY_MS = 30000;   // let Wi-Fi connect first

// ---------- boot (PCB) ----------
constexpr float    WIFI_START_MIN_V      = 13.0f;   // wait for the reservoir before Wi-Fi
                                                    // (line sags to ~16 V when cooling)
constexpr uint32_t WIFI_START_TIMEOUT_MS = 20000;

// ---------- reporting ----------
constexpr uint32_t AUX_PUBLISH_MS  = 10000;   // AUX and VRES: mean, min and max over this period
constexpr uint32_t RSSI_PUBLISH_MS = 30000;
constexpr uint32_t MQTT_RETRY_MS   = 5000;
constexpr uint32_t UPTIME_PUBLISH_MS = 60000;

// ---------- watchdogs ----------
constexpr uint32_t WIFI_RADIO_RESTART_MS = 60UL * 1000UL;        // down this long: restart the radio
constexpr uint32_t WIFI_REBOOT_MS        = 5UL * 60UL * 1000UL;  // down this long: reboot
constexpr uint32_t LOOP_WDT_TIMEOUT_MS   = 30000;                // main loop stuck this long: reboot

enum Mode { MODE_COOL, MODE_HEAT, MODE_FAN, MODE_AUTO };
enum Fan  { FAN_LOW, FAN_MEDIUM, FAN_HIGH };
const char* MODE_NAMES[] = {"Cool", "Heat", "Fan only", "Auto"};
constexpr int MODE_COUNT = 4;
const char* FAN_NAMES[]  = {"Low", "Medium", "High"};

bool powerOn = false;
Mode mode = MODE_COOL;
Fan  fan  = FAN_LOW;

volatile uint8_t frameBits = 0;        // what is being sent (read by the TX task)
bool compRunning = false, runHeat = false;
uint32_t compOffSince = 0, compOnSince = 0;

// Thermostat settings (saved).
bool thermostatOn = false;
float setpoint = SETPOINT_DEFAULT, hysteresis = HYST_DEFAULT, deadband = DEADBAND_DEFAULT;
int minRunMin = MIN_RUN_DEFAULT_MIN;
String roomName[MAX_ROOMS], roomSensor[MAX_ROOMS];
String selectedRoom;                   // a room name; empty = Average
// Thermostat state.
RoomReading readings[MAX_ROOMS] = {};
bool haOnline = true;                  // false after HA's "offline" on homeassistant/status
bool tstatIdle = false;
Direction tstatDir = Direction::None;
TstatStatus tstatStatus = TstatStatus::Off;
float tstatActual = NAN;
bool roomsDirty = false;               // room list changed: republish it once it settles
uint32_t roomsChangedAt = 0;

WiFiClient net;
PubSubClient mqtt(net);
Preferences prefs;
String nodeId, base;                   // e.g. "aircon_3c6105145424"

// Survive a software/watchdog reboot (not a power cut), so the reason can be reported afterwards.
constexpr uint32_t RTC_MAGIC = 0xA1C0FFEE;
RTC_NOINIT_ATTR uint32_t rtcMagic;
RTC_NOINIT_ATTR uint32_t rtcWifiReboot;     // 1 = the Wi-Fi watchdog rebooted us
RTC_NOINIT_ATTR uint32_t rtcLastDisc;       // last Wi-Fi disconnect reason code

String restartReason;
volatile uint8_t lastDiscReason = 0;   // 0 = none since power-on
uint32_t wifiReconnects = 0;           // Wi-Fi lost and regained since boot
uint32_t wifiDownSince = 0, lastRadioRestart = 0;
bool wifiEverConnected = false;

inline bool before(uint32_t now, uint32_t t) { return (int32_t)(now - t) < 0; }

// ---------- analog sampling ----------
// A task reads AUX, VRES and (PCB) the tablet output at ~1 kHz. Every 10 ms it updates the
// short-term readings the tablet manager uses, and adds to the window that loop() publishes
// every AUX_PUBLISH_MS. The window's min and max are single samples, so brief dips and spikes
// show up in them.
struct Window {
  float auxSum, auxMin, auxMax;
  float vresSum, vresMin;
  float vtabSum;
  uint32_t n;
  void reset() { *this = {0, INFINITY, -INFINITY, 0, INFINITY, 0, 0}; }
  static Window take();                // hands over the published window and starts a new one
};
portMUX_TYPE sampleMux = portMUX_INITIALIZER_UNLOCKED;
Window window;
volatile float vresNow = 0, vtabNow = 0;   // 10 ms means

float readVolts(int pin, float ratio) { return analogReadMilliVolts(pin) / 1000.0f * ratio; }

void samplerTask(void*) {
  for (;;) {
    Window b;
    b.reset();
    for (int i = 0; i < 10; i++) {
      float a = readVolts(PIN_AUX, AUX_RATIO);
      float v = readVolts(PIN_VRES, VRES_RATIO);
      b.auxSum += a; b.auxMin = min(b.auxMin, a); b.auxMax = max(b.auxMax, a);
      b.vresSum += v; b.vresMin = min(b.vresMin, v);
#if BOARD_PCB
      b.vtabSum += readVolts(PIN_VTAB, VTAB_RATIO);
#endif
      b.n++;
      vTaskDelay(1);
    }
    portENTER_CRITICAL(&sampleMux);
    window.auxSum += b.auxSum;
    window.auxMin = min(window.auxMin, b.auxMin);
    window.auxMax = max(window.auxMax, b.auxMax);
    window.vresSum += b.vresSum;
    window.vresMin = min(window.vresMin, b.vresMin);
    window.vtabSum += b.vtabSum;
    window.n += b.n;
    portEXIT_CRITICAL(&sampleMux);
    vresNow = b.vresSum / b.n;
    vtabNow = b.vtabSum / b.n;
  }
}

Window Window::take() {
  portENTER_CRITICAL(&sampleMux);
  Window w = window;
  window.reset();
  portEXIT_CRITICAL(&sampleMux);
  return w;
}

// ---------- control ----------
uint32_t lockoutRemainingMs() {
  uint32_t off = millis() - compOffSince;
  return off >= COMPRESSOR_MIN_OFF_MS ? 0 : COMPRESSOR_MIN_OFF_MS - off;
}

// The selected room's index, or -1 for Average (also when the name isn't a configured room).
int selectedRoomIndex() {
  if (selectedRoom.isEmpty()) return -1;
  for (int i = 0; i < MAX_ROOMS; i++) if (readings[i].configured && roomName[i] == selectedRoom) return i;
  return -1;
}

void updateThermostat() {
  uint32_t now = millis();
  tstatActual = actualTemperature(readings, selectedRoomIndex(), now, ROOM_STALE_MS, haOnline);
  bool anyRooms = false;
  for (int i = 0; i < MAX_ROOMS; i++) anyRooms |= readings[i].configured;
  const TstatMode modes[] = {TstatMode::Cool, TstatMode::Heat, TstatMode::FanOnly, TstatMode::Auto};
  TstatInputs in = {
    powerOn, thermostatOn, modes[mode], anyRooms, tstatActual,
    setpoint, hysteresis, deadband,
    compRunning && now - compOnSince < (uint32_t)minRunMin * 60000UL,
  };
  TstatResult r = thermostatStep(tstatIdle, in, tstatDir);
  tstatIdle = r.idle;
  tstatStatus = r.status;
  tstatDir = r.dir;
}

uint8_t requestedBits() {
  if (!powerOn || tstatIdle) return 0;
  uint8_t b = fan == FAN_HIGH ? BIT_FAN_HIGH : fan == FAN_MEDIUM ? BIT_FAN_MED : BIT_FAN_LOW;
  bool heat = mode == MODE_HEAT || (mode == MODE_AUTO && tstatDir == Direction::Heat);
  bool cool = mode == MODE_COOL || (mode == MODE_AUTO && tstatDir == Direction::Cool);
  if (mode == MODE_AUTO && !heat && !cool) return 0;   // no direction yet (the thermostat idles it anyway)
  if (cool) b |= COMP_BITS;
  if (heat) b |= COMP_BITS | HEAT_BIT;
  return b;
}

// Runs the thermostat, applies the compressor lockout and builds the frame that is actually sent.
void updateFrame() {
  updateThermostat();
  uint8_t req = requestedBits();
  bool wantComp = req & COMP_BITS;
  bool wantHeat = req & HEAT_BIT;
  if (compRunning && (!wantComp || wantHeat != runHeat)) {   // stop, incl. heat <-> cool
    compRunning = false;
    compOffSince = millis();
  } else if (!compRunning && wantComp && lockoutRemainingMs() == 0) {
    compRunning = true;
    compOnSince = millis();
    runHeat = wantHeat;
  }
  uint8_t b = req & ~(COMP_BITS | HEAT_BIT);
  if (compRunning) b |= req & (COMP_BITS | HEAT_BIT);
  frameBits = b;
}

// Called after a command from HA. Rooms are saved separately, only when they change, since HA
// re-sends them every minute.
void saveSettings() {
  prefs.putUChar("mode", mode);
  prefs.putUChar("fan", fan);
  prefs.putBool("tstat", thermostatOn);
  prefs.putFloat("setpt", setpoint);
  prefs.putFloat("hyst", hysteresis);
  prefs.putFloat("deadb", deadband);
  prefs.putUChar("minrun", minRunMin);
  prefs.putString("roomsel", selectedRoom);
}

String roomKey(int i, char field) { return String("r") + i + field; }

void saveRooms() {
  for (int i = 0; i < MAX_ROOMS; i++) {
    if (prefs.getString(roomKey(i, 'n').c_str(), "") != roomName[i]) prefs.putString(roomKey(i, 'n').c_str(), roomName[i]);
    if (prefs.getString(roomKey(i, 's').c_str(), "") != roomSensor[i]) prefs.putString(roomKey(i, 's').c_str(), roomSensor[i]);
  }
}

void loadSettings() {
  mode = (Mode)min<uint8_t>(prefs.getUChar("mode", MODE_COOL), MODE_AUTO);
  fan  = (Fan)min<uint8_t>(prefs.getUChar("fan", FAN_LOW), FAN_HIGH);
  thermostatOn = prefs.getBool("tstat", false);
  setpoint   = constrain(prefs.getFloat("setpt", SETPOINT_DEFAULT), SETPOINT_MIN, SETPOINT_MAX);
  hysteresis = constrain(prefs.getFloat("hyst", HYST_DEFAULT), HYST_MIN, HYST_MAX);
  deadband   = constrain(prefs.getFloat("deadb", DEADBAND_DEFAULT), DEADBAND_MIN, DEADBAND_MAX);
  minRunMin  = min<int>(prefs.getUChar("minrun", MIN_RUN_DEFAULT_MIN), MIN_RUN_MAX_MIN);
  selectedRoom = prefs.getString("roomsel", "");
  for (int i = 0; i < MAX_ROOMS; i++) {
    roomName[i] = prefs.getString(roomKey(i, 'n').c_str(), "");
    roomSensor[i] = prefs.getString(roomKey(i, 's').c_str(), "");
    readings[i].configured = !roomName[i].isEmpty();
  }
}

// Exact timing via RMT, 1 us ticks.
void txTask(void*) {
  rmt_data_t sym[9];
  TickType_t last = xTaskGetTickCount();
  for (;;) {
    uint8_t bits = frameBits;
    for (int i = 0; i < 9; i++) {
      sym[i].level0 = 1;  sym[i].duration0 = PULSE_US;
      sym[i].level1 = 0;
      sym[i].duration1 = (i < 8)
          ? (((bits >> i) & 1) ? LONG_GAP_US : SHORT_GAP_US) - PULSE_US
          : 100;
    }
    rmtWrite(PIN_TX, sym, 9, RMT_WAIT_FOR_EVER);
    vTaskDelayUntil(&last, pdMS_TO_TICKS(FRAME_PERIOD_MS));
  }
}

// ---------- tablet output (PCB) ----------
#if BOARD_PCB
bool tabWanted = true;                 // Home Assistant switch; not saved, so on after every boot
bool tabOn = false;
uint32_t tabOffUntil = 0, vresOkSince = 0, vresLowSince = 0, vtabLowSince = 0;
uint32_t tabShedCount = 0, tabFaultCount = 0;
const char* tabStatus = "Waiting: boot";
float vresAtBoot = 0;
uint32_t wifiWaitMs = 0;               // how long boot waited for the reservoir

void setTablet(bool on) {
  tabOn = on;
  digitalWrite(PIN_TAB_EN, on ? HIGH : LOW);
  vtabLowSince = vresLowSince = 0;
}

void updateTablet() {
  uint32_t now = millis();
  float vres = vresNow, vtab = vtabNow;

  if (!tabWanted) { if (tabOn) setTablet(false); tabStatus = "Disabled"; return; }

  if (tabOn) {
    // Load shedding: the ESP32 always has priority over the tablet.
    if (vres < TAB_SHED_V) {
      if (!vresLowSince) vresLowSince = now;
      if (now - vresLowSince >= TAB_SHED_MS) {
        setTablet(false); tabShedCount++; tabOffUntil = now + TAB_RETRY_MS;
        tabStatus = "Shed: VRES low"; return;
      }
    } else vresLowSince = 0;
    // Short or overload on the cable: output collapses while the limiter holds ~87 mA.
    if (vtab < TAB_FAULT_V) {
      if (!vtabLowSince) vtabLowSince = now;
      if (now - vtabLowSince >= TAB_FAULT_MS) {
        setTablet(false); tabFaultCount++; tabOffUntil = now + TAB_RETRY_MS;
        tabStatus = "Fault: output low (short or overload?)"; return;
      }
    } else vtabLowSince = 0;
    tabStatus = "On";
    return;
  }

  // Currently off: decide whether to switch on.
  if (now < TAB_BOOT_DELAY_MS)  { tabStatus = "Waiting: boot"; return; }
  if (before(now, tabOffUntil)) { return; }          // keep the shed/fault message
  if (vres >= TAB_ON_MIN_V) {
    if (!vresOkSince) vresOkSince = now;
    if (now - vresOkSince >= TAB_ON_SETTLE_MS) { setTablet(true); vresOkSince = 0; tabStatus = "On"; }
    else tabStatus = "Waiting: VRES settling";
  } else { vresOkSince = 0; tabStatus = "Waiting: VRES too low"; }
}
#endif

// ---------- MQTT / Home Assistant ----------
String topic(const char* t) { return base + "/" + t; }

void publish(const char* t, const String& v, bool retain = true) {
  mqtt.publish(topic(t).c_str(), v.c_str(), retain);
}

// One Home Assistant discovery message. `extra` holds the entity-specific JSON fields.
void discover(const char* component, const char* objectId, const char* name, const String& extra) {
  String dev = "{\"ids\":[\"" + nodeId + "\"],\"name\":\"Aircon\",\"mf\":\"DIY\","
               "\"mdl\":\"" + String(MODEL) + "\",\"sw\":\"aircon_controller\"}";
  String p = "{\"name\":\"" + String(name) + "\",\"uniq_id\":\"" + nodeId + "_" + objectId + "\","
             "\"avty_t\":\"" + topic("status") + "\",\"dev\":" + dev + "," + extra + "}";
  String t = String("homeassistant/") + component + "/" + nodeId + "/" + objectId + "/config";
  mqtt.publish(t.c_str(), p.c_str(), true);
}

// Removes an entity that older versions of this sketch announced.
void undiscover(const char* component, const char* objectId) {
  String t = String("homeassistant/") + component + "/" + nodeId + "/" + objectId + "/config";
  mqtt.publish(t.c_str(), "", true);
}

String options(const char* const* names, int n) {
  String s = "[";
  for (int i = 0; i < n; i++) s += (i ? ",\"" : "\"") + String(names[i]) + "\"";
  return s + "]";
}

void discoverVoltage(const char* objectId, const char* name) {
  discover("sensor", objectId, name,
           "\"stat_t\":\"" + topic(objectId) + "\",\"unit_of_meas\":\"V\",\"dev_cla\":\"voltage\","
           "\"stat_cla\":\"measurement\",\"sug_dsp_prc\":2,\"ent_cat\":\"diagnostic\"");
}

String jsonEscape(const String& s) {
  String out;
  out.reserve(s.length() + 2);
  for (unsigned int i = 0; i < s.length(); i++) {
    char c = s[i];
    if (c == '"' || c == '\\') { out += '\\'; out += c; }
    else if ((uint8_t)c < 0x20) out += ' ';
    else out += c;
  }
  return out;
}

// "Average" plus each configured room's name (duplicates left out).
String roomOptions() {
  String s = "[\"Average\"";
  for (int i = 0; i < MAX_ROOMS; i++) {
    if (!readings[i].configured || roomName[i] == "Average") continue;
    bool dup = false;
    for (int j = 0; j < i; j++) dup |= readings[j].configured && roomName[j] == roomName[i];
    if (!dup) s += ",\"" + jsonEscape(roomName[i]) + "\"";
  }
  return s + "]";
}

// Sent again whenever the room list changes.
void discoverThermostatRoom() {
  discover("select", "thermostat_room", "Thermostat room",
           "\"cmd_t\":\"" + topic("thermostat_room/set") + "\",\"stat_t\":\"" + topic("thermostat_room") + "\","
           "\"options\":" + roomOptions() + ",\"icon\":\"mdi:home-thermometer\"");
}

void publishDiscovery() {
  discover("switch", "power", "Power",
           "\"cmd_t\":\"" + topic("power/set") + "\",\"stat_t\":\"" + topic("power") + "\",\"icon\":\"mdi:power\"");
  discover("switch", "thermostat", "Thermostat",
           "\"cmd_t\":\"" + topic("thermostat/set") + "\",\"stat_t\":\"" + topic("thermostat") + "\",\"icon\":\"mdi:thermostat-auto\"");
  discover("number", "setpoint", "Set point",
           "\"cmd_t\":\"" + topic("setpoint/set") + "\",\"stat_t\":\"" + topic("setpoint") + "\","
           "\"min\":" + String(SETPOINT_MIN, 1) + ",\"max\":" + String(SETPOINT_MAX, 1) + ",\"step\":0.5,"
           "\"mode\":\"box\",\"unit_of_meas\":\"\\u00b0C\",\"dev_cla\":\"temperature\"");
  discoverThermostatRoom();
  discover("number", "hysteresis", "Thermostat hysteresis",
           "\"cmd_t\":\"" + topic("hysteresis/set") + "\",\"stat_t\":\"" + topic("hysteresis") + "\","
           "\"min\":" + String(HYST_MIN, 1) + ",\"max\":" + String(HYST_MAX, 1) + ",\"step\":0.1,"
           "\"mode\":\"box\",\"unit_of_meas\":\"\\u00b0C\",\"ent_cat\":\"config\",\"icon\":\"mdi:arrow-expand-vertical\"");
  discover("number", "deadband", "Auto deadband",
           "\"cmd_t\":\"" + topic("deadband/set") + "\",\"stat_t\":\"" + topic("deadband") + "\","
           "\"min\":" + String(DEADBAND_MIN, 1) + ",\"max\":" + String(DEADBAND_MAX, 1) + ",\"step\":0.1,"
           "\"mode\":\"box\",\"unit_of_meas\":\"\\u00b0C\",\"ent_cat\":\"config\",\"icon\":\"mdi:arrow-left-right\"");
  discover("number", "min_run", "Minimum run time",
           "\"cmd_t\":\"" + topic("min_run/set") + "\",\"stat_t\":\"" + topic("min_run") + "\","
           "\"min\":0,\"max\":" + String(MIN_RUN_MAX_MIN) + ",\"step\":1,"
           "\"mode\":\"box\",\"unit_of_meas\":\"min\",\"dev_cla\":\"duration\",\"ent_cat\":\"config\"");
  undiscover("binary_sensor", "actual_power");   // replaced by Operation
  discover("sensor", "operation", "Operation",
           "\"stat_t\":\"" + topic("operation") + "\",\"dev_cla\":\"enum\","
           "\"options\":[\"Off\",\"Heating\",\"Cooling\",\"Fan\"],\"icon\":\"mdi:hvac\"");
  discover("sensor", "auto_direction", "Auto direction",
           "\"stat_t\":\"" + topic("auto_direction") + "\",\"icon\":\"mdi:swap-vertical\"");
  discover("sensor", "thermostat_temperature", "Thermostat temperature",
           "\"stat_t\":\"" + topic("thermostat_temperature") + "\",\"unit_of_meas\":\"\\u00b0C\","
           "\"dev_cla\":\"temperature\",\"stat_cla\":\"measurement\",\"sug_dsp_prc\":1");
  discover("sensor", "thermostat_status", "Thermostat status",
           "\"stat_t\":\"" + topic("thermostat_status") + "\",\"icon\":\"mdi:thermostat\"");
  discover("sensor", "rooms", "Rooms",
           "\"stat_t\":\"" + topic("rooms") + "\",\"json_attr_t\":\"" + topic("rooms_attr") + "\","
           "\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:home-group\"");
  discover("select", "mode", "Mode",
           "\"cmd_t\":\"" + topic("mode/set") + "\",\"stat_t\":\"" + topic("mode") + "\","
           "\"options\":" + options(MODE_NAMES, MODE_COUNT) + ",\"icon\":\"mdi:thermostat\"");
  discover("select", "fan", "Fan speed",
           "\"cmd_t\":\"" + topic("fan/set") + "\",\"stat_t\":\"" + topic("fan") + "\","
           "\"options\":" + options(FAN_NAMES, 3) + ",\"icon\":\"mdi:fan\"");
  discover("binary_sensor", "compressor", "Compressor",
           "\"stat_t\":\"" + topic("compressor") + "\",\"dev_cla\":\"running\"");
  discoverVoltage("aux", "AUX voltage");
  discoverVoltage("aux_min", "AUX minimum");
  discoverVoltage("aux_max", "AUX maximum");
  discoverVoltage("vres", "VRES");
  discoverVoltage("vres_min", "VRES minimum");
  discover("sensor", "rssi", "Wi-Fi signal",
           "\"stat_t\":\"" + topic("rssi") + "\",\"unit_of_meas\":\"dBm\",\"dev_cla\":\"signal_strength\","
           "\"stat_cla\":\"measurement\",\"ent_cat\":\"diagnostic\"");
  discover("sensor", "lockout", "Compressor lockout",
           "\"stat_t\":\"" + topic("lockout") + "\",\"unit_of_meas\":\"s\",\"dev_cla\":\"duration\","
           "\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:timer-sand\"");
  discover("sensor", "uptime", "Uptime",
           "\"stat_t\":\"" + topic("uptime") + "\",\"unit_of_meas\":\"s\",\"dev_cla\":\"duration\","
           "\"stat_cla\":\"total_increasing\",\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:timer-outline\"");
  discover("sensor", "restart_reason", "Last restart reason",
           "\"stat_t\":\"" + topic("restart_reason") + "\",\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:restart\"");
  discover("sensor", "wifi_reconnects", "Wi-Fi reconnects",
           "\"stat_t\":\"" + topic("wifi_reconnects") + "\",\"stat_cla\":\"total_increasing\","
           "\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:wifi-refresh\"");
  discover("sensor", "wifi_disconnect", "Last Wi-Fi disconnect",
           "\"stat_t\":\"" + topic("wifi_disconnect") + "\",\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:wifi-alert\"");
#if BOARD_PCB
  discover("switch", "tablet", "Tablet output",
           "\"cmd_t\":\"" + topic("tablet/set") + "\",\"stat_t\":\"" + topic("tablet") + "\","
           "\"ent_cat\":\"config\",\"icon\":\"mdi:tablet\"");
  discover("sensor", "tablet_status", "Tablet output status",
           "\"stat_t\":\"" + topic("tablet_status") + "\",\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:tablet\"");
  discoverVoltage("tablet_v", "Tablet output voltage");
  discover("sensor", "tablet_sheds", "Tablet output sheds",
           "\"stat_t\":\"" + topic("tablet_sheds") + "\",\"stat_cla\":\"total_increasing\","
           "\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:flash-off\"");
  discover("sensor", "tablet_faults", "Tablet output faults",
           "\"stat_t\":\"" + topic("tablet_faults") + "\",\"stat_cla\":\"total_increasing\","
           "\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:alert\"");
  discoverVoltage("vres_at_boot", "VRES at boot");
  discover("sensor", "boot_wifi_wait", "Boot Wi-Fi wait",
           "\"stat_t\":\"" + topic("boot_wifi_wait") + "\",\"unit_of_meas\":\"ms\",\"dev_cla\":\"duration\","
           "\"ent_cat\":\"diagnostic\",\"icon\":\"mdi:timer-sand\"");
#endif
}

// ---------- diagnostics ----------
String resetReasonName(esp_reset_reason_t r, bool wifiWatchdog) {
  switch (r) {
    case ESP_RST_POWERON:  return "Power on";
    case ESP_RST_BROWNOUT: return "Brownout (supply dip)";
    case ESP_RST_SW:       return wifiWatchdog ? "Wi-Fi watchdog" : "Software (e.g. OTA update)";
    case ESP_RST_PANIC:    return "Crash";
    case ESP_RST_INT_WDT:
    case ESP_RST_TASK_WDT:
    case ESP_RST_WDT:      return "Watchdog (loop hung)";
    case ESP_RST_EXT:      return "Reset button";
    case ESP_RST_DEEPSLEEP:return "Deep sleep";
    default:               return "Other (" + String((int)r) + ")";
  }
}

String discReasonName(uint8_t r) {
  if (!r) return "No disconnects";     // not "None": HA's MQTT sensor reads that as unknown
  return String(WiFi.STA.disconnectReasonName((wifi_err_reason_t)r)) + " (" + r + ")";
}

uint8_t publishedDisc = 255;
uint32_t publishedReconnects = UINT32_MAX;

void publishDiagnostics(bool force) {
  static uint32_t lastUptime = 0;
  uint32_t now = millis();
  if (force) {
    publish("restart_reason", restartReason);
#if BOARD_PCB
    publish("vres_at_boot", String(vresAtBoot, 2));
    publish("boot_wifi_wait", String(wifiWaitMs));
#endif
  }
  if (force || now - lastUptime >= UPTIME_PUBLISH_MS) {
    lastUptime = now;
    publish("uptime", String(now / 1000));
  }
  if (force || wifiReconnects != publishedReconnects) {
    publishedReconnects = wifiReconnects;
    publish("wifi_reconnects", String(wifiReconnects));
  }
  uint8_t d = lastDiscReason;
  if (force || d != publishedDisc) {
    publishedDisc = d;
    publish("wifi_disconnect", discReasonName(d));
  }
}

int lastLockoutS = -1, lastComp = -1;

void publishRooms() {
  String attrs = "{\"rooms\":[";
  int n = 0;
  for (int i = 0; i < MAX_ROOMS; i++) {
    if (!readings[i].configured) continue;
    attrs += String(n++ ? "," : "") + "{\"name\":\"" + jsonEscape(roomName[i]) + "\",\"sensor\":\"" + jsonEscape(roomSensor[i]) + "\"}";
  }
  attrs += "]}";
  publish("rooms_attr", attrs);
  publish("rooms", String(n));
}

void publishControls() {
  publish("power", powerOn ? "ON" : "OFF");
  publish("mode", MODE_NAMES[mode]);
  publish("fan", FAN_NAMES[fan]);
  publish("thermostat", thermostatOn ? "ON" : "OFF");
  publish("setpoint", String(setpoint, 1));
  publish("hysteresis", String(hysteresis, 1));
  publish("deadband", String(deadband, 1));
  publish("min_run", String(minRunMin));
  publish("thermostat_room", selectedRoomIndex() < 0 ? "Average" : selectedRoom);
#if BOARD_PCB
  publish("tablet", tabWanted ? "ON" : "OFF");
#endif
}

void publishStatus(bool force) {
  if (force || (int)compRunning != lastComp) {
    lastComp = compRunning;
    publish("compressor", compRunning ? "ON" : "OFF");
  }
  static const char* lastOperation = nullptr;
  uint8_t bits = frameBits;
  const char* operation = !bits ? "Off"
                        : !(bits & COMP_BITS) ? "Fan"          // fan only, or the compressor locked out
                        : (bits & HEAT_BIT) ? "Heating" : "Cooling";
  if (force || operation != lastOperation) { lastOperation = operation; publish("operation", operation); }
  static int lastDir = -1;
  if (force || (int)tstatDir != lastDir) {
    lastDir = (int)tstatDir;
    publish("auto_direction", directionName(tstatDir));
  }
  static String lastTemp;
  String t = isnan(tstatActual) ? String("None") : String(tstatActual, 1);   // "None" = unknown in HA
  if (force || t != lastTemp) { lastTemp = t; publish("thermostat_temperature", t); }
  static int lastTstat = -1;
  if (force || (int)tstatStatus != lastTstat) {
    lastTstat = (int)tstatStatus;
    publish("thermostat_status", tstatStatusName(tstatStatus));
  }
  int s = (lockoutRemainingMs() + 999) / 1000;
  if (force || s != lastLockoutS) {
    lastLockoutS = s;
    publish("lockout", String(s));
  }
#if BOARD_PCB
  static const char* lastStatus = nullptr;
  static uint32_t lastSheds = UINT32_MAX, lastFaults = UINT32_MAX;
  if (force || tabStatus != lastStatus) { lastStatus = tabStatus; publish("tablet_status", tabStatus); }
  if (force || tabShedCount != lastSheds) { lastSheds = tabShedCount; publish("tablet_sheds", String(tabShedCount)); }
  if (force || tabFaultCount != lastFaults) { lastFaults = tabFaultCount; publish("tablet_faults", String(tabFaultCount)); }
#endif
}

int indexOf(const char* const* names, int n, const String& v) {
  for (int i = 0; i < n; i++) if (v.equalsIgnoreCase(names[i])) return i;
  return -1;
}

// Parses a number, rejecting anything that isn't one ("unavailable", "unknown", "").
bool parseNumber(const String& v, float& out) {
  if (v.isEmpty()) return false;
  char* end;
  float f = strtof(v.c_str(), &end);
  if (*end != '\0' || !isfinite(f)) return false;
  out = f;
  return true;
}

bool parseOnOff(const String& v, bool& out) {
  if (v.equalsIgnoreCase("ON")) out = true;
  else if (v.equalsIgnoreCase("OFF")) out = false;
  else return false;
  return true;
}

// <base>/room/<1-4>/name, /sensor (retained, from the blueprint) and /temperature.
void onRoomMessage(const String& rest, const String& v) {
  int slash = rest.indexOf('/');
  if (slash != 1) return;
  int i = rest[0] - '1';
  if (i < 0 || i >= MAX_ROOMS) return;
  String field = rest.substring(slash + 1);
  if (field == "temperature") {
    readings[i].valid = parseNumber(v, readings[i].temp);
    readings[i].at = millis();
    updateFrame();
    return;
  }
  String* target = field == "name" ? &roomName[i] : field == "sensor" ? &roomSensor[i] : nullptr;
  if (!target) return;
  String value = v.substring(0, field == "name" ? ROOM_NAME_MAX : ROOM_SENSOR_MAX);
  if (*target == value) return;              // HA re-sends these every minute
  *target = value;
  bool configured = !roomName[i].isEmpty();
  if (configured != readings[i].configured || field == "sensor") readings[i] = {configured, false, 0, 0};
  saveRooms();
  roomsDirty = true;
  roomsChangedAt = millis();
  updateFrame();
}

void onMessage(char* t, byte* payload, unsigned int len) {
  String tp(t), v;
  v.reserve(len);
  for (unsigned int i = 0; i < len; i++) v += (char)payload[i];
  v.trim();

  if (tp == "homeassistant/status") {
    haOnline = v != "offline";
    if (v == "online") { publishDiscovery(); publishControls(); publishRooms(); publishStatus(true); publishDiagnostics(true); }   // HA restarted: send everything again
    updateFrame();
    publishStatus(false);
    return;
  }
  String roomPrefix = topic("room/");
  if (tp.startsWith(roomPrefix)) {
    onRoomMessage(tp.substring(roomPrefix.length()), v);
    publishStatus(false);
    return;
  }
  if (tp == topic("power/set")) {
    if (!parseOnOff(v, powerOn)) return;
  } else if (tp == topic("thermostat/set")) {
    if (!parseOnOff(v, thermostatOn)) return;
  } else if (tp == topic("setpoint/set")) {
    float f;
    if (!parseNumber(v, f)) return;
    setpoint = constrain(f, SETPOINT_MIN, SETPOINT_MAX);
  } else if (tp == topic("hysteresis/set")) {
    float f;
    if (!parseNumber(v, f)) return;
    hysteresis = constrain(f, HYST_MIN, HYST_MAX);
  } else if (tp == topic("deadband/set")) {
    float f;
    if (!parseNumber(v, f)) return;
    deadband = constrain(f, DEADBAND_MIN, DEADBAND_MAX);
  } else if (tp == topic("min_run/set")) {
    float f;
    if (!parseNumber(v, f)) return;
    minRunMin = constrain((int)lroundf(f), 0, MIN_RUN_MAX_MIN);
  } else if (tp == topic("thermostat_room/set")) {
    if (v.equalsIgnoreCase("Average")) selectedRoom = "";
    else {
      selectedRoom = v;
      if (selectedRoomIndex() < 0) { selectedRoom = ""; publishControls(); return; }   // not a room
    }
  } else if (tp == topic("mode/set")) {
    int i = indexOf(MODE_NAMES, MODE_COUNT, v);
    if (i < 0) return;
    if (mode != (Mode)i) { tstatIdle = false; tstatDir = Direction::None; }   // decide afresh
    mode = (Mode)i;
  } else if (tp == topic("fan/set")) {
    int i = indexOf(FAN_NAMES, 3, v);
    if (i < 0) return;
    fan = (Fan)i;
#if BOARD_PCB
  } else if (tp == topic("tablet/set")) {
    if (v.equalsIgnoreCase("ON")) { tabWanted = true; tabOffUntil = millis(); }   // also ends a retry wait
    else if (v.equalsIgnoreCase("OFF")) tabWanted = false;
    else return;
    updateTablet();
    publishControls();
    publishStatus(false);
    return;
#endif
  } else {
    return;
  }
  saveSettings();
  updateFrame();
  publishControls();
  publishStatus(false);
}

void mqttConnect() {
  static uint32_t lastTry = 0;
  if (mqtt.connected() || WiFi.status() != WL_CONNECTED) return;
  if (lastTry && millis() - lastTry < MQTT_RETRY_MS) return;
  lastTry = millis();
  bool ok = strlen(MQTT_USER)
      ? mqtt.connect(nodeId.c_str(), MQTT_USER, MQTT_PASS, topic("status").c_str(), 0, true, "offline")
      : mqtt.connect(nodeId.c_str(), topic("status").c_str(), 0, true, "offline");
  if (!ok) return;
  publish("status", "online");
  publishDiscovery();
  publishControls();
  publishRooms();
  publishStatus(true);
  publishDiagnostics(true);
  for (const char* t : {"power/set", "mode/set", "fan/set", "thermostat/set", "setpoint/set", "hysteresis/set",
                        "deadband/set", "min_run/set", "thermostat_room/set", "room/+/name", "room/+/sensor", "room/+/temperature"}) {
    mqtt.subscribe(topic(t).c_str());
  }
#if BOARD_PCB
  mqtt.subscribe(topic("tablet/set").c_str());
#endif
  mqtt.subscribe("homeassistant/status");
}

// ---------- Wi-Fi and watchdogs ----------
void onWifiDisconnected(arduino_event_id_t, arduino_event_info_t info) {
  uint8_t r = info.wifi_sta_disconnected.reason;
  if (r == WIFI_REASON_ASSOC_LEAVE) return;   // we disconnected on purpose (radio restart)
  lastDiscReason = r;
  rtcLastDisc = r;
}

void startWifi() {
  WiFi.mode(WIFI_STA);
  WiFi.setHostname(HOSTNAME);
  WiFi.setSleep(true);                 // lower average current when self-powered
  WiFi.setAutoReconnect(true);         // the core retries most disconnects; wifiWatchdog() covers the rest
  WiFi.begin(WIFI_SSID, WIFI_PASS);
  WiFi.setTxPower(WIFI_POWER_8_5dBm);  // lower peak current; raise if the signal is weak
}

// Restarts the radio after 1 minute without Wi-Fi, and reboots after 5 minutes.
// The TX task keeps sending frames throughout, except for ~1 s during a reboot.
void wifiWatchdog(uint32_t now) {
  if (WiFi.status() == WL_CONNECTED) {
    if (wifiDownSince && wifiEverConnected) wifiReconnects++;
    wifiDownSince = 0;
    wifiEverConnected = true;
    return;
  }
  if (!wifiDownSince) wifiDownSince = now ? now : 1;
  uint32_t down = now - wifiDownSince;
  if (down >= WIFI_REBOOT_MS) {
    rtcWifiReboot = 1;
    ESP.restart();
  }
  if (down >= WIFI_RADIO_RESTART_MS && now - lastRadioRestart >= WIFI_RADIO_RESTART_MS) {
    lastRadioRestart = now;
    mqtt.disconnect();
    WiFi.disconnect(true);
    WiFi.mode(WIFI_OFF);
    delay(200);
    startWifi();
  }
}

// Reboots if loop() stops running for LOOP_WDT_TIMEOUT_MS. Paused during OTA updates,
// which run inside ArduinoOTA.handle() for longer than that.
void startLoopWatchdog() {
  esp_task_wdt_config_t cfg = {
    .timeout_ms = LOOP_WDT_TIMEOUT_MS,
    .idle_core_mask = 0,
    .trigger_panic = true,
  };
  esp_task_wdt_reconfigure(&cfg);
  enableLoopWDT();
}

// ---------- setup / loop ----------
void setup() {
  esp_reset_reason_t rr = esp_reset_reason();
  if (rtcMagic != RTC_MAGIC) {         // power-on: RTC memory holds garbage
    rtcMagic = RTC_MAGIC;
    rtcWifiReboot = 0;
    rtcLastDisc = 0;
  }
  restartReason = resetReasonName(rr, rr == ESP_RST_SW && rtcWifiReboot);
  rtcWifiReboot = 0;
  lastDiscReason = rtcLastDisc;        // carried over a watchdog reboot

#if BOARD_PCB
  pinMode(PIN_TAB_EN, OUTPUT);
  digitalWrite(PIN_TAB_EN, LOW);       // tablet output off until power is healthy
  analogSetPinAttenuation(PIN_VTAB, ADC_11db);
#endif
  pinMode(PIN_LED, OUTPUT);
  digitalWrite(PIN_LED, LOW);
  analogSetPinAttenuation(PIN_AUX, ADC_11db);
  analogSetPinAttenuation(PIN_VRES, ADC_11db);

  prefs.begin("aircon", false);
  powerOn = false;                     // always start off, whatever was set before the reboot
  prefs.remove("power");               // saved by older versions of this sketch
  loadSettings();

  rmtInit(PIN_TX, RMT_TX_MODE, RMT_MEM_NUM_BLOCKS_1, 1000000);
  rmtSetEOT(PIN_TX, LOW);              // idle low = transistor off
  compOffSince = millis();             // compressor lockout from boot
  updateFrame();
  xTaskCreate(txTask, "tx", 4096, nullptr, configMAX_PRIORITIES - 1, nullptr);
  window.reset();
  xTaskCreate(samplerTask, "sample", 4096, nullptr, 1, nullptr);

#if BOARD_PCB
  // Let the reservoir charge before Wi-Fi's current bursts.
  vresAtBoot = readVolts(PIN_VRES, VRES_RATIO);
  uint32_t t0 = millis();
  while (readVolts(PIN_VRES, VRES_RATIO) < WIFI_START_MIN_V && millis() - t0 < WIFI_START_TIMEOUT_MS) delay(100);
  wifiWaitMs = millis() - t0;
#endif

  // Factory MAC from eFuse: unique per chip and readable before Wi-Fi starts
  // (WiFi.macAddress() returns zeros until the station interface is up).
  uint64_t efuse = ESP.getEfuseMac();
  uint8_t mac[6];
  for (int i = 0; i < 6; i++) mac[i] = (efuse >> (8 * i)) & 0xFF;
  char id[32];
  snprintf(id, sizeof(id), "aircon_%02x%02x%02x%02x%02x%02x", mac[0], mac[1], mac[2], mac[3], mac[4], mac[5]);
  nodeId = id;
  base = nodeId;

  WiFi.onEvent(onWifiDisconnected, ARDUINO_EVENT_WIFI_STA_DISCONNECTED);
  startWifi();

  ArduinoOTA.setHostname(HOSTNAME);
  if (strlen(OTA_PASS)) ArduinoOTA.setPassword(OTA_PASS);
  ArduinoOTA.onStart([] { disableLoopWDT(); });
  ArduinoOTA.onError([](ota_error_t) { enableLoopWDT(); });
  ArduinoOTA.begin();

  mqtt.setServer(MQTT_HOST, MQTT_PORT);
  mqtt.setBufferSize(1536);            // discovery messages are larger than the 256-byte default
  mqtt.setSocketTimeout(5);            // don't block for long if the broker stops answering
  mqtt.setCallback(onMessage);

  startLoopWatchdog();
}

void loop() {
  static uint32_t lastCtl = 0, lastAux = 0, lastRssi = 0;

  ArduinoOTA.handle();
  mqttConnect();
  mqtt.loop();
  digitalWrite(PIN_LED, mqtt.connected() ? HIGH : LOW);
#if BOARD_PCB
  updateTablet();
#endif

  uint32_t now = millis();
  if (now - lastCtl >= 1000) {         // ends lockouts; reports compressor and lockout changes
    lastCtl = now;
    updateFrame();
    wifiWatchdog(now);
    if (mqtt.connected()) { publishStatus(false); publishDiagnostics(false); }
  }
  if (roomsDirty && now - roomsChangedAt >= ROOMS_SETTLE_MS) {
    // A selected room that's gone falls back to Average.
    if (!selectedRoom.isEmpty() && selectedRoomIndex() < 0) { selectedRoom = ""; saveSettings(); }
    if (mqtt.connected()) {
      roomsDirty = false;
      discoverThermostatRoom();
      publishRooms();
      publishControls();
    }
  }
  if (now - lastAux >= AUX_PUBLISH_MS) {
    lastAux = now;
    Window w = Window::take();
    if (w.n && mqtt.connected()) {
      publish("aux", String(w.auxSum / w.n, 2));
      publish("aux_min", String(w.auxMin, 2));
      publish("aux_max", String(w.auxMax, 2));
      publish("vres", String(w.vresSum / w.n, 2));
      publish("vres_min", String(w.vresMin, 2));
#if BOARD_PCB
      publish("tablet_v", String(w.vtabSum / w.n, 2));
#endif
    }
  }
  if (now - lastRssi >= RSSI_PUBLISH_MS) {
    lastRssi = now;
    if (mqtt.connected() && WiFi.status() == WL_CONNECTED) publish("rssi", String(WiFi.RSSI()));
  }
  delay(2);
}
