#ifndef NEWFLASHER_H
#define NEWFLASHER_H
#include <stddef.h>
#include <stdint.h>

enum FastbootHeader {
    FR_ERROR = 0,
    FR_OKAY,
    FR_DATA,
    FR_FAIL,
    FR_NO_HEADER,
};

struct CVec {
    char *ptr;
    size_t len;
    size_t capacity;
};

struct FastbootDevice {
    char _inner[8 + 8 + 128 + 112];
    struct CVec vec;
};

typedef struct FastbootDevice *HANDLE;

unsigned int file_size(char *filename);

static int file_exist(char *file);

static void remove_file_exist(char *file);

/* Parse an octal number, ignoring leading and trailing nonsense. */
int parseoct(const char *p, size_t n);

/* Verify the tar checksum. */
int verify_checksum(const char *p);

/* Returns true if this is 512 zero bytes. */
bool is_end_of_archive(const char *p);

void trim(char *ptr);


enum FastbootHeader get_reply_ffi(HANDLE handle);

bool get_data_ffi(HANDLE handle, const char *cmd);
bool fastboot_cmd_ffi(HANDLE handle, const char *cmd, char *reply_buf, size_t reply_buf_len);
bool fastboot_download_ffi(HANDLE handle, const char *data, size_t len);
uint32_t getvar_u32_ffi(HANDLE handle, const char *cmd, uint32_t fallback);


struct TrimArea {
	uint8_t partition;
	size_t unit;
	uint8_t *data;
    size_t size;
};

#endif //NEWFLASHER_H
