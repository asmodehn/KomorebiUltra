/* Test-only driver for the probe's compositor app_id events.
 *
 * Compiled together with probe.c (with its main renamed) and the generated
 * Wayland protocol sources. Every Wayland entry point used by probe.c is a
 * local stub: no compositor connection, surface, input or artifact is created.
 * One fake toplevel is registered, then either the foreign-toplevel app_id
 * event or the COSMIC toplevel app_id event is delivered with the value under
 * test; the probe's own main emits the row data.
 */
#define main probe_main_unused
#include "probe.c"
#undef main

#include <poll.h>
#include <stdarg.h>

#define FAKE_BOUND ((struct wl_proxy *)(uintptr_t)0x3)
#define FAKE_FT_HANDLE ((struct ext_foreign_toplevel_handle_v1 *)(uintptr_t)0x4)
#define FAKE_CT_HANDLE ((struct zcosmic_toplevel_handle_v1 *)(uintptr_t)0x5)

static void (**saved_ft_impl)(void);
static void *saved_ft_data;
static int fail_connect;
static int drive_cosmic;
static const char *pending_value;

struct wl_display *wl_display_connect(const char *name)
{ (void)name; return fail_connect ? NULL : (struct wl_display *)(uintptr_t)0x1; }
int wl_display_roundtrip(struct wl_display *d) { (void)d; return 0; }
int wl_display_get_fd(struct wl_display *d) { (void)d; return -1; }
int wl_display_dispatch(struct wl_display *d) { (void)d; return 0; }
int poll(struct pollfd *fds, nfds_t nfds, int timeout)
{ (void)fds; (void)nfds; (void)timeout; return 0; }
uint32_t wl_proxy_get_version(struct wl_proxy *p) { (void)p; return 1; }
struct wl_proxy *wl_proxy_marshal_flags(struct wl_proxy *p, uint32_t opcode,
                                        const struct wl_interface *iface,
                                        uint32_t version, uint32_t flags, ...)
{
    (void)p; (void)opcode; (void)version; (void)flags;
    return iface == &zcosmic_toplevel_handle_v1_interface
        ? (struct wl_proxy *)FAKE_CT_HANDLE : FAKE_BOUND;
}
int wl_proxy_add_listener(struct wl_proxy *p, void (**impl)(void), void *data)
{
    if (p == (struct wl_proxy *)FAKE_FT_HANDLE) {
        saved_ft_impl = impl;
        saved_ft_data = data;
        if (pending_value && !drive_cosmic) {
            ((struct ext_foreign_toplevel_handle_v1_listener *)impl)->app_id(
                data, FAKE_FT_HANDLE, pending_value);
        }
    } else if (p == (struct wl_proxy *)FAKE_CT_HANDLE && pending_value
               && drive_cosmic) {
        ((struct zcosmic_toplevel_handle_v1_listener *)impl)->app_id(
            data, FAKE_CT_HANDLE, pending_value);
    }
    return 0;
}

int main(int argc, char **argv)
{
    if (argc == 2 && strcmp(argv[1], "--no-compositor") == 0) {
        fail_connect = 1;
    } else if (argc == 2 && strcmp(argv[1], "--no-cosmic") == 0) {
        /* No compositor globals: probe_main reports NO_COSMIC_TOPLEVEL_INFO. */
    } else if (argc == 3 && (strcmp(argv[1], "--foreign") == 0
                             || strcmp(argv[1], "--cosmic") == 0)) {
        drive_cosmic = strcmp(argv[1], "--cosmic") == 0;
        pending_value = argv[2];
    } else {
        return 64;
    }

    if (pending_value) {
        on_global(NULL, (struct wl_registry *)FAKE_BOUND, 1,
                  "ext_foreign_toplevel_list_v1", 1);
        on_global(NULL, (struct wl_registry *)FAKE_BOUND, 2,
                  "zcosmic_toplevel_info_v1", 3);
        ft_toplevel(NULL, NULL, FAKE_FT_HANDLE);
        if (!saved_ft_impl) return 65;
        tops[0].x = 8; tops[0].y = 40; tops[0].w = 3424; tops[0].h = 1316;
    }
    return probe_main_unused();
}
