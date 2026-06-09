#ifndef NEWFLASHER_H
#define NEWFLASHER_H
#include <stddef.h>
#include <stdint.h>

struct RustVec {
    uint8_t *ptr;
    size_t len;
    size_t capacity;
};

struct RustVec new_cvec(size_t capacity);

enum FastbootReply {
    FR_ERROR = 0,
    FR_NO_HEADER,
    FR_OKAY,
    FR_DATA,
    FR_FAIL,
};

struct usb_handle {
    int desc;
    unsigned char ep_in;
    unsigned char ep_out;
    char _context[8 + 8 + 128 + 112];
};

typedef struct usb_handle *HANDLE;

unsigned int file_size(char *filename);

static int file_exist(char *file);

static void remove_file_exist(char *file);

void trim(char *ptr);

bool fastboot_cmd_ffi(HANDLE handle, struct RustVec *cvec, const char *var, char *str_buf, size_t len);

bool fastboot_download_ffi(HANDLE handle, struct RustVec *cvec, const char *data, size_t len);

uint32_t getvar_u32_ffi(HANDLE handle, struct RustVec *cvec, const char *var, uint32_t fallback);

#endif //NEWFLASHER_H
