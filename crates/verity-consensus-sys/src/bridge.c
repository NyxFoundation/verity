#include <lean/lean.h>
#include <pthread.h>
#include <stdint.h>
#include <string.h>

extern void lean_finalize_thread(void);
extern void lean_initialize_runtime_module(void);
extern void lean_initialize_thread(void);
extern lean_object *initialize_ssz_VeritySsz(uint8_t builtin);
extern lean_object *verity_ssz_hash_tree_root_lean(uint8_t tag, lean_object *encoded);

static pthread_once_t init_once = PTHREAD_ONCE_INIT;
static pthread_key_t thread_key;
static int32_t init_status = 1;

static void finalize_thread(void *initialized) {
  if (initialized != NULL) {
    lean_finalize_thread();
  }
}

static void initialize_module(void) {
  lean_initialize_runtime_module();
  if (pthread_key_create(&thread_key, finalize_thread) != 0) {
    return;
  }
  lean_object *result = initialize_ssz_VeritySsz(1);
  if (lean_io_result_is_error(result)) {
    lean_dec_ref(result);
    return;
  }
  lean_dec_ref(result);
  lean_io_mark_end_initialization();
  pthread_setspecific(thread_key, (void *)1);
  init_status = 0;
}

static int32_t initialize_calling_thread(void) {
  if (pthread_once(&init_once, initialize_module) != 0 || init_status != 0) {
    return 1;
  }
  if (pthread_getspecific(thread_key) == NULL) {
    lean_initialize_thread();
    if (pthread_setspecific(thread_key, (void *)1) != 0) {
      lean_finalize_thread();
      return 1;
    }
  }
  return 0;
}

int32_t verity_ssz_hash_tree_root(
    uint8_t tag,
    const uint8_t *data,
    size_t len,
    uint8_t out[32]) {
  if (initialize_calling_thread() != 0 || (data == NULL && len != 0) || out == NULL) {
    return 1;
  }

  lean_object *array = lean_alloc_sarray(1, len, len);
  if (len != 0) {
    memcpy(lean_sarray_cptr(array), data, len);
  }
  lean_object *result = verity_ssz_hash_tree_root_lean(tag, array);
  if (lean_sarray_size(result) != 32) {
    lean_dec_ref(result);
    return 2;
  }

  memcpy(out, lean_sarray_cptr(result), 32);
  lean_dec_ref(result);
  return 0;
}
