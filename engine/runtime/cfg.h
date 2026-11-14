// Settings: the runtime's environment variables (documented as KLM_*, named TOTK_* in the code), with the settings file
// next to the executable (kalamar.ini, else totk.ini) behind them.
#pragma once
#include <stdint.h>

// getenv for a setting: the environment variable when set, else the totk.ini key (the name without "TOTK_", any
// case), else the device profile's default (cfg.c: the Steam Deck), else the low-memory profile's (cfg_low_memory), else NULL. Same string semantics as getenv; the pointer stays valid for the process (a value cfg_set replaces
// is kept). A name without the TOTK_ prefix is read from the environment only (cfg_env). Safe from any thread once cfg_init has returned.
const char *cfg_get(const char *name);

// The one reader of the environment for the runtime's variables. The code names a variable by its older name,
// TOTK_<NAME> or SPLAT3_<NAME>; the documentation names it KLM_<NAME>, which is read first, the older name after (any
// other name: getenv). Same string semantics as getenv. tests/env_names_check.py fails on a documented variable read or
// set around these two.
const char *cfg_env(const char *name);
// The one writer, for this process and the processes it starts: sets KLM_<NAME> for an older name and removes the
// older copy, so that cfg_env answers value whatever the inherited environment held (value NULL removes both). 1 on
// success.
int cfg_setenv(const char *name, const char *value);

// Reads totk.ini once, from mem_init after the address space is claimed and before any setting is read. Logs the file, its key count, unknown keys and environment overrides.
void cfg_init(void);
// 1 once cfg_init has returned: a thread started before it (discord_rpc.c's) waits for this before reading settings.
int cfg_ready(void);

// Replaces the loaded keys with those of the file at path (tests). 1 when the file was read, 0 when missing.
// Not thread-safe: nothing else may call cfg_get meanwhile.
int cfg_load(const char *path);

// Live settings (the in-game menu). cfg_set stores value as the totk.ini value of name (TOTK_ prefixed), marks it for
// cfg_save and bumps cfg_version. cfg_get returns it unless the environment sets name. Any thread.
void cfg_set(const char *name, const char *value);
// Bumped by every cfg_set. A consumer that keeps a parsed value compares this with the number it last saw (one atomic
// load) and parses again when it changed.
unsigned cfg_version(void);
// 1 when the environment sets name: cfg_set cannot change what cfg_get returns (the menu greys the setting out).
int cfg_from_env(const char *name);
// 1 when the player set name (the environment or totk.ini), 0 when cfg_get gives a device profile's default or NULL.
int cfg_is_set(const char *name);
// The device profile in use, "deck" or "desktop" (TOTK_DEVICE_PROFILE, else what the machine is).
const char *cfg_device_profile(void);
// The low-memory profile (TOTK_LOW_MEMORY, cfg.c): 1 when its defaults apply. cfg_note_vram gives it the size of the
// GPU's largest device-local heap in MiB, once the device exists.
int cfg_low_memory(void);
void cfg_note_vram(uint64_t mib);
// Writes the keys cfg_set changed since the last save into totk.ini next to totk.exe (cfg_save_to: the file at path).
// Only those lines change: the last line of a changed key gets the new value, a missing key goes under its section
// (after the commented example line of the key when the section has one; the section is added at the end of the file
// when missing), comments and every other line stay byte for byte. 1 on success or with nothing to write.
int cfg_save(void);
int cfg_save_to(const char *path);

// Reset (the menu's "Reset all settings to defaults"). Removes the active line of each key (names without TOTK_, any
// case) from totk.ini at the next cfg_save and its value from memory, so the consumer's own default applies again; the
// commented example lines stay. Keys the environment sets are skipped. Bumps cfg_version. Returns how many it reset.
int cfg_reset(const char *const *keys, int n);
// 0 when the last cfg_save / cfg_save_to failed to write (the values stay set in memory, the next save tries again).
int cfg_last_save_ok(void);
// Every setting with a value, for a bug report ("Copy system info"): the totk.ini and cfg_set keys (name without
// TOTK_, upper case; from_env 0), then the known TOTK_ variables the environment sets (from_env 1).
void cfg_each_active(void (*cb)(const char *key, const char *value, int from_env, void *ctx), void *ctx);

// What the menu knows about a setting. key: without TOTK_, upper case. def: what the consumer uses when the key is
// unset ("" = no fixed default, the consumer picks per machine). values: "0|1", "mailbox|fifo|immediate", a range
// "0..300", or NULL for free text. live: applies at once; the others are read at start.
typedef struct { const char *key, *def, *values; int live; } CfgMeta;
// The row of key (prefix and case ignored); NULL for a key the menu does not show.
const CfgMeta *cfg_meta(const char *key);
// The i-th row (0-based, table order); NULL past the last.
const CfgMeta *cfg_meta_at(int i);

// The setting's value for consumers: the environment or totk.ini value, else the CfgMeta default. name has the TOTK_
// prefix, like cfg_get; a name without a row is a bug (rt_fatal). A row with an empty default has no fixed value: its
// consumer reads cfg_get and keeps its own "unset" branch. An empty value counts as unset.
const char *cfg_value(const char *name);
// cfg_value parsed. A set value that does not parse (int: whole string; double: strtod) gives the default's number.
// cfg_bool: "0" is false, anything else true.
int cfg_int(const char *name);
double cfg_double(const char *name);
int cfg_bool(const char *name);
