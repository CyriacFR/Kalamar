// Save-data trace (savetrace.c), TOTK_SAVE_TRACE=<dir>: the file calls fs_host.c reports to it.
#pragma once
#include <stdint.h>

int savetrace_on(void);                                                      // the trace is on (reads the setting once)
void savetrace_fs_open(const char *path, uint64_t h, int mode, uint32_t result);   // OpenFile on a host file
void savetrace_fs_io(uint64_t h, int write, int64_t offset, const void *buf, uint64_t size);   // bytes moved
void savetrace_fs_close(uint64_t h);
void savetrace_fs_op(const char *op, const char *path, int64_t arg, uint32_t result);   // create, delete, rename, commit
