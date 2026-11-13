// Copy this file to secrets.h (which git ignores) and fill in your own values.
#pragma once

const char* WIFI_SSID = "your-ssid";
const char* WIFI_PASS = "your-password";
const char* MQTT_HOST = "your-mqtt-broker";   // e.g. Home Assistant's IP, with the Mosquitto add-on
const char* MQTT_USER = "";                   // "" if the broker allows anonymous clients
const char* MQTT_PASS = "";
const char* OTA_PASS  = "";                   // set to require a password for Wi-Fi updates
