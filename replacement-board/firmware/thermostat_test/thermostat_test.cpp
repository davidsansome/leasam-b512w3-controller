// Host-side checks for ../aircon_controller/thermostat.h. Build and run on a PC:
//   g++ -std=c++17 -Wall -o thermostat_test thermostat_test.cpp && ./thermostat_test

#include <cstdio>
#include <cstdlib>

#include "../aircon_controller/thermostat.h"

static int failures = 0;
#define CHECK(cond) do { if (!(cond)) { std::printf("FAIL line %d: %s\n", __LINE__, #cond); failures++; } } while (0)

static TstatInputs cooling(float actual) {
  TstatInputs in = {};
  in.power = true; in.thermostat = true; in.anyRooms = true;
  in.actual = actual; in.setpoint = 22.0f; in.hysteresis = 0.5f; in.deadband = 1.0f;
  return in;
}

int main() {
  // Averages fresh rooms only; a selected room is used alone.
  RoomReading rooms[MAX_ROOMS] = {
    {true, true, 20.0f, 1000}, {true, true, 22.0f, 1000}, {true, false, 0, 1000}, {false, true, 30.0f, 1000},
  };
  CHECK(actualTemperature(rooms, -1, 2000, 300000, true) == 21.0f);
  CHECK(actualTemperature(rooms, 1, 2000, 300000, true) == 22.0f);
  CHECK(isnan(actualTemperature(rooms, 2, 2000, 300000, true)));        // selected room has no reading
  CHECK(isnan(actualTemperature(rooms, -1, 2000, 300000, false)));      // HA offline
  CHECK(isnan(actualTemperature(rooms, -1, 1000 + 300000, 300000, true)));   // stale

  // Cooling: runs down to the set point, idles until set point + hysteresis.
  TstatResult r = thermostatStep(false, cooling(23.0f));
  CHECK(!r.idle && r.status == TstatStatus::Running);
  r = thermostatStep(false, cooling(22.0f));
  CHECK(r.idle && r.status == TstatStatus::AtSetPoint);
  r = thermostatStep(true, cooling(22.4f));
  CHECK(r.idle);
  r = thermostatStep(true, cooling(22.5f));
  CHECK(!r.idle);
  r = thermostatStep(false, cooling(22.2f));                           // in the band, running: keeps running
  CHECK(!r.idle);

  // Heating mirrors it.
  TstatInputs heat = cooling(22.0f);
  heat.mode = TstatMode::Heat;
  CHECK(thermostatStep(false, heat).idle);
  heat.actual = 21.6f;
  CHECK(thermostatStep(true, heat).idle);
  heat.actual = 21.5f;
  CHECK(!thermostatStep(true, heat).idle);

  // Minimum run time holds it on, but doesn't stop it idling once it's already idle.
  TstatInputs held = cooling(21.0f);
  held.minRunHeld = true;
  r = thermostatStep(false, held);
  CHECK(!r.idle && r.status == TstatStatus::MinRun);
  CHECK(thermostatStep(true, held).idle);

  // No temperature or no rooms: idle.
  r = thermostatStep(false, cooling(NAN));
  CHECK(r.idle && r.status == TstatStatus::NoTemperature);
  TstatInputs none = cooling(21.0f);
  none.anyRooms = false;
  r = thermostatStep(false, none);
  CHECK(r.idle && r.status == TstatStatus::NoRooms);


  // Auto (set point 22, deadband 1, hysteresis 0.5): starts on the temperature's side of
  // the set point, cools down to 23 and heats up to 21.
  TstatInputs au = cooling(24.0f);
  au.mode = TstatMode::Auto;
  r = thermostatStep(false, au);
  CHECK(!r.idle && r.status == TstatStatus::Running && r.dir == Direction::Cool);
  au.actual = 23.0f;
  r = thermostatStep(false, au, Direction::Cool);
  CHECK(r.idle && r.status == TstatStatus::NearSetPoint && r.dir == Direction::Cool);
  au.actual = 23.4f;
  CHECK(thermostatStep(true, au, Direction::Cool).idle);              // within the hysteresis
  au.actual = 23.5f;
  CHECK(!thermostatStep(true, au, Direction::Cool).idle);
  au.actual = 22.2f;                                                   // just above the set point
  r = thermostatStep(false, au);
  CHECK(r.dir == Direction::Cool && r.idle);
  au.actual = 21.8f;                                                   // just below it
  r = thermostatStep(false, au);
  CHECK(r.dir == Direction::Heat && r.idle);
  au.actual = 21.0f;
  r = thermostatStep(false, au, Direction::Heat);
  CHECK(r.idle && r.dir == Direction::Heat);

  // Keeps its direction (idle) across the deadband; switches past the far resume point.
  au.actual = 23.4f;
  r = thermostatStep(true, au, Direction::Heat);
  CHECK(r.idle && r.dir == Direction::Heat);
  au.actual = 23.5f;
  r = thermostatStep(true, au, Direction::Heat);
  CHECK(!r.idle && r.dir == Direction::Cool && r.status == TstatStatus::Running);
  au.actual = 20.6f;
  r = thermostatStep(true, au, Direction::Cool);
  CHECK(r.idle && r.dir == Direction::Cool);
  au.actual = 20.5f;
  r = thermostatStep(true, au, Direction::Cool);
  CHECK(!r.idle && r.dir == Direction::Heat);

  // Switching direction while running isn't held by the minimum run time.
  au.actual = 20.0f; au.minRunHeld = true;
  r = thermostatStep(false, au, Direction::Cool);
  CHECK(!r.idle && r.dir == Direction::Heat && r.status == TstatStatus::Running);
  au.minRunHeld = false;

  // No temperature: idle, direction kept (or none yet).
  au.actual = NAN;
  r = thermostatStep(false, au, Direction::Heat);
  CHECK(r.idle && r.status == TstatStatus::NoTemperature && r.dir == Direction::Heat);
  r = thermostatStep(false, au);
  CHECK(r.idle && r.dir == Direction::None);

  // Auto with the thermostat off (continuous) idles; power off clears the direction.
  au.actual = 25.0f; au.thermostat = false;
  r = thermostatStep(false, au, Direction::Cool);
  CHECK(r.idle && r.status == TstatStatus::AutoNeedsThermostat);
  au.thermostat = true; au.power = false;
  r = thermostatStep(true, au, Direction::Cool);
  CHECK(!r.idle && r.status == TstatStatus::Off && r.dir == Direction::None);

  // Outside Auto there's no direction.
  CHECK(thermostatStep(false, cooling(25.0f), Direction::Heat).dir == Direction::None);

  // Off, continuous and fan-only never idle, and clear a previous idle.
  TstatInputs in = cooling(10.0f);
  in.mode = TstatMode::FanOnly;
  CHECK(!thermostatStep(true, in).idle && thermostatStep(true, in).status == TstatStatus::FanOnly);
  in = cooling(10.0f); in.thermostat = false;
  CHECK(!thermostatStep(true, in).idle && thermostatStep(true, in).status == TstatStatus::Continuous);
  in = cooling(10.0f); in.power = false;
  CHECK(!thermostatStep(true, in).idle && thermostatStep(true, in).status == TstatStatus::Off);

  if (failures) { std::printf("%d failed\n", failures); return 1; }
  std::printf("all passed\n");
  return 0;
}
