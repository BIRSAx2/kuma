#ifndef KUMA_H
#define KUMA_H

#include <stddef.h>
#include <stdint.h>

#ifdef __cplusplus
extern "C" {
#endif

typedef uint32_t KumaTarget;

#define KUMA_TARGET_AMD64_SYSV ((KumaTarget)1u)
#define KUMA_TARGET_AMD64_APPLE ((KumaTarget)2u)
#define KUMA_TARGET_AARCH64_ELF ((KumaTarget)3u)
#define KUMA_TARGET_AARCH64_APPLE ((KumaTarget)4u)

typedef enum KumaStatus {
    KUMA_STATUS_SUCCESS = 0,
    KUMA_STATUS_INVALID_ARGUMENT = 1,
    KUMA_STATUS_INVALID_UTF8 = 2,
    KUMA_STATUS_PARSE_ERROR = 3,
    KUMA_STATUS_INVALID_IR = 4,
    KUMA_STATUS_UNSUPPORTED_TARGET = 5,
    KUMA_STATUS_IO_ERROR = 6,
    KUMA_STATUS_TOOLCHAIN_ERROR = 7,
    KUMA_STATUS_INTERNAL_ERROR = 8
} KumaStatus;

typedef struct KumaBuffer {
    uint8_t *data;
    size_t length;
    size_t capacity;
} KumaBuffer;

KumaStatus kuma_compile(
    const uint8_t *input,
    size_t input_length,
    KumaTarget target,
    KumaBuffer *assembly,
    KumaBuffer *error);

KumaStatus kuma_assemble(
    const uint8_t *input,
    size_t input_length,
    KumaTarget target,
    const char *output_object,
    KumaBuffer *error);

KumaStatus kuma_compile_and_link(
    const uint8_t *input,
    size_t input_length,
    KumaTarget target,
    const char *output_path,
    const char *const *extra_objects,
    size_t extra_object_count,
    KumaBuffer *error);

void kuma_buffer_free(KumaBuffer *buffer);

#ifdef __cplusplus
}
#endif

#endif
