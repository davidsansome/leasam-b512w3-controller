// Thermostat decisions, kept apart from MQTT and the hardware so they're easy to follow.
//
// The thermostat only ever switches the whole unit off ("idle") and back on. Power, mode
// and fan keep the values the user chose. In Auto it also picks the direction (heat or cool).

#pragma once
#include <math.h>
#include <stdint.h>

constexpr int MAX_ROOMS = 4;

struct RoomReading {
  bool configured;     // has a name (rooms without one are unused)
  bool valid;          // the last reading was a number...
  float temp;
  uint32_t at;         // ...received at this millis()
};

// The temperature the thermostat works from: the selected room's (`selected` 0..3), or with
// `selected` < 0 the average of every configured room with a fresh reading. NAN if there's
// nothing usable. A reading is fresh if it's valid, under `staleMs` old and HA is online.
inline float actualTemperature(const RoomReading* rooms, int selected, uint32_t now, uint32_t staleMs, bool haOnline) {
  auto fresh = [&](const RoomReading& r) {
    return haOnline && r.configured && r.valid && now - r.at < staleMs;
  };
  if (selected >= 0 && selected < MAX_ROOMS) return fresh(rooms[selected]) ? rooms[selected].temp : NAN;
  float sum = 0;
  int n = 0;
  for (int i = 0; i < MAX_ROOMS; i++) {
    if (fresh(rooms[i])) { sum += rooms[i].temp; n++; }
  }
  return n ? sum / n : NAN;
}

enum class TstatMode { Cool, Heat, FanOnly, Auto };

// Which way Auto is working. None outside Auto, and in Auto until there's a temperature.
enum class Direction { None, Heat, Cool };

inline const char* directionName(Direction d) {
  switch (d) {
    case Direction::Heat: return "Heat";
    case Direction::Cool: return "Cool";
    case Direction::None: return "None";   // "None" = unknown in HA
  }
  return "None";
}

enum class TstatStatus {
  Off,              // power off
  Continuous,       // thermostat off: runs whatever the temperature
  FanOnly,          // the thermostat has nothing to do in fan-only mode
  Running,
  MinRun,           // at the set point, but held on until the minimum run time is up
  AtSetPoint,       // idle
  NearSetPoint,     // idle: Auto, within the deadband
  NoTemperature,    // idle
  NoRooms,          // idle
  AutoNeedsThermostat,   // idle: Auto with the thermostat off (continuous)
};

inline const char* tstatStatusName(TstatStatus s) {
  switch (s) {
    case TstatStatus::Off:                 return "Off";
    case TstatStatus::Continuous:          return "Continuous";
    case TstatStatus::FanOnly:             return "Fan only";
    case TstatStatus::Running:             return "Running";
    case TstatStatus::MinRun:              return "Running: minimum run time";
    case TstatStatus::AtSetPoint:          return "Idle: at set point";
    case TstatStatus::NearSetPoint:        return "Idle: near set point";
    case TstatStatus::NoTemperature:       return "Idle: no temperature";
    case TstatStatus::NoRooms:             return "Idle: no rooms configured";
    case TstatStatus::AutoNeedsThermostat: return "Idle: Auto needs the thermostat";
  }
  return "?";
}

struct TstatInputs {
  bool power;          // the Power switch
  bool thermostat;     // the Thermostat switch (off = continuous)
  TstatMode mode;
  bool anyRooms;       // at least one room is configured
  float actual;        // actualTemperature(), NAN if none
  float setpoint;
  float hysteresis;
  float deadband;      // Auto: heats to set point - deadband, cools to set point + deadband
  bool minRunHeld;     // the compressor started less than the minimum run time ago
};

struct TstatResult {
  bool idle;
  TstatStatus status;
  Direction dir;       // Auto's direction, to pass back in next time
};

// Auto's direction. It starts on the side of the set point the temperature is on, and
// switches only once the temperature is past the other side's resume point (set point ∓
// deadband ∓ hysteresis), so it can't flip back and forth near the set point.
inline Direction autoDirection(Direction dir, const TstatInputs& in) {
  if (isnan(in.actual)) return dir;
  float margin = in.deadband + in.hysteresis;
  if (dir == Direction::None) return in.actual > in.setpoint ? Direction::Cool : Direction::Heat;
  if (dir == Direction::Heat && in.actual >= in.setpoint + margin) return Direction::Cool;
  if (dir == Direction::Cool && in.actual <= in.setpoint - margin) return Direction::Heat;
  return dir;
}

// One step of the thermostat. `idle` is the previous decision: between the set point and
// set point ± hysteresis the unit keeps doing what it was doing. `dir` is the previous
// direction in Auto.
inline TstatResult thermostatStep(bool idle, const TstatInputs& in, Direction dir = Direction::None) {
  if (!in.power) return {false, TstatStatus::Off, Direction::None};
  if (in.mode != TstatMode::Auto) dir = Direction::None;
  if (in.mode == TstatMode::Auto && !in.thermostat) return {true, TstatStatus::AutoNeedsThermostat, dir};
  if (!in.thermostat) return {false, TstatStatus::Continuous, dir};
  if (in.mode == TstatMode::FanOnly) return {false, TstatStatus::FanOnly, dir};

  bool heating = in.mode == TstatMode::Heat;
  float target = in.setpoint;
  TstatStatus idleWhy = TstatStatus::AtSetPoint;
  if (in.mode == TstatMode::Auto) {
    Direction next = autoDirection(dir, in);
    if (next != dir && dir != Direction::None) idle = true;   // the new direction decides afresh
    dir = next;
    heating = dir == Direction::Heat;
    target = heating ? in.setpoint - in.deadband : in.setpoint + in.deadband;
    idleWhy = TstatStatus::NearSetPoint;
  }

  TstatStatus why;
  bool wantIdle;
  if (!in.anyRooms) {
    wantIdle = true; why = TstatStatus::NoRooms;
  } else if (isnan(in.actual)) {
    wantIdle = true; why = TstatStatus::NoTemperature;
  } else {
    bool reached = heating ? in.actual >= target : in.actual <= target;
    bool resume  = heating ? in.actual <= target - in.hysteresis
                           : in.actual >= target + in.hysteresis;
    wantIdle = idle ? !resume : reached;
    why = idleWhy;
  }

  if (!wantIdle) return {false, TstatStatus::Running, dir};
  if (!idle && in.minRunHeld) return {false, TstatStatus::MinRun, dir};
  return {true, why, dir};
}
