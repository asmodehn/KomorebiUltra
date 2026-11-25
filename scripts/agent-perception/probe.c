/* Read-only desktop window probe for the developer perception map.
 *
 * Connects to the compositor as an ordinary Wayland client, reads the
 * ext-foreign-toplevel-list and COSMIC toplevel-info protocols, and prints one
 * ROW line per observed toplevel. It creates no surface, injects no input and
 * prints no title, opaque identifier, path, URL or pixel data.
 */
#define _GNU_SOURCE
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <poll.h>
#include <wayland-client.h>
#include "ext-foreign-toplevel-list-v1.h"
#include "cosmic-toplevel-info-unstable-v1.h"
#include "cosmic-workspace-unstable-v1.h"
#include "ext-workspace-v1.h"

#define MAX_TOPS 256
#define APP_LEN 64
struct top {
    struct ext_foreign_toplevel_handle_v1 *ft;
    char app[APP_LEN];
    int has_app;
    int invalid_app;
    int geom_count, state_count;
    int x, y, w, h;
    int activated, fullscreen, maximized, minimized;
};
static struct top tops[MAX_TOPS];
static int ntops, ft_ready, cosmic_ready, cosmic_done;
static struct top v1_probe_top;
static struct zcosmic_toplevel_info_v1 *cosmic_mgr;

/* Both compositor app_id events pass through this one filter. A blob-like
 * identity is never emitted, even when followed by otherwise-allowed
 * characters; the probe reports INVALID_ROW instead of inventing a shorter
 * identity. */
static int is_hex(char c)
{
    return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'f') ||
           (c >= 'A' && c <= 'F');
}
static int is_alnum(char c)
{
    return (c >= '0' && c <= '9') || (c >= 'a' && c <= 'z') ||
           (c >= 'A' && c <= 'Z');
}
static int hex_group(const char *s, size_t offset, size_t n)
{
    for (size_t i = 0; i < n; i++)
        if (!is_hex(s[offset + i])) return 0;
    return 1;
}
static int blob_like(const char *s)
{
    size_t n = strlen(s);
    /* Hyphen-separated UUID shape (8-4-4-4-12 hex digits), with any suffix. */
    for (size_t i = 0; i + 36 <= n; i++) {
        if (hex_group(s + i, 0, 8) && s[i + 8] == '-' &&
            hex_group(s + i, 9, 4) && s[i + 13] == '-' &&
            hex_group(s + i, 14, 4) && s[i + 18] == '-' &&
            hex_group(s + i, 19, 4) && s[i + 23] == '-' &&
            hex_group(s + i, 24, 12))
            return 1;
    }
    for (size_t i = 0; i < n; i++) {
        size_t hex = 0, alnum = 0;
        while (i + hex < n && is_hex(s[i + hex])) hex++;
        while (i + alnum < n && is_alnum(s[i + alnum])) alnum++;
        if (hex >= 32 || alnum >= 40) return 1;
    }
    for (size_t i = 0; i + 4 <= n; i++) {
        if (s[i] != 'A' || (s[i + 1] != 'K' && s[i + 1] != 'S') ||
            s[i + 2] != 'I' || s[i + 3] != 'A') continue;
        size_t j = i + 4;
        while (j < n && ((s[j] >= 'A' && s[j] <= 'Z') ||
                         (s[j] >= '0' && s[j] <= '9'))) j++;
        if (j - (i + 4) >= 12) return 1;
    }
    return 0;
}
static void set_app(struct top *t, const char *s)
{
    if (!s) return;
    size_t n = 0;
    for (; s[n] && n + 1 < sizeof(t->app); n++) {
        unsigned char c = (unsigned char)s[n];
        int ok = (c >= 'a' && c <= 'z') || (c >= 'A' && c <= 'Z') ||
                 (c >= '0' && c <= '9') || c == '.' || c == '_' || c == '-';
        if (!ok) break;
        t->app[n] = (char)c;
    }
    t->app[n] = '\0';
    if (blob_like(t->app)) {
        t->app[0] = '\0';
        t->has_app = 0;
        t->invalid_app = 1;
        return;
    }
    t->has_app = n > 0;
}

static void ft_closed(void *d, struct ext_foreign_toplevel_handle_v1 *h) { (void)d; (void)h; }
static void ft_done(void *d, struct ext_foreign_toplevel_handle_v1 *h) { (void)d; (void)h; }
static void ft_title(void *d, struct ext_foreign_toplevel_handle_v1 *h, const char *s)
{ (void)d; (void)h; (void)s; }
static void ft_ident(void *d, struct ext_foreign_toplevel_handle_v1 *h, const char *s)
{ (void)d; (void)h; (void)s; }
static void ft_finished(void *d, struct ext_foreign_toplevel_list_v1 *m) { (void)d; (void)m; }

static void ct_closed(void *d, struct zcosmic_toplevel_handle_v1 *h) { (void)d; (void)h; }
static void ct_done(void *d, struct zcosmic_toplevel_handle_v1 *h) { (void)d; (void)h; }
static void ct_title(void *d, struct zcosmic_toplevel_handle_v1 *h, const char *s)
{ (void)d; (void)h; (void)s; }
static void ct_app(void *d, struct zcosmic_toplevel_handle_v1 *h, const char *s)
{ (void)h; set_app(d, s); }
static void ct_output(void *d, struct zcosmic_toplevel_handle_v1 *h, struct wl_output *o)
{ (void)d; (void)h; (void)o; }
static void ct_ws_leave_v1(void *d, struct zcosmic_toplevel_handle_v1 *h,
                           struct zcosmic_workspace_handle_v1 *w)
{ (void)d; (void)h; (void)w; }
static void ct_ws_enter_ext(void *d, struct zcosmic_toplevel_handle_v1 *h,
                            struct ext_workspace_handle_v1 *w)
{ (void)d; (void)h; (void)w; }
static void ct_ws_leave_ext(void *d, struct zcosmic_toplevel_handle_v1 *h,
                            struct ext_workspace_handle_v1 *w)
{ (void)d; (void)h; (void)w; }
static void info_finished(void *d, struct zcosmic_toplevel_info_v1 *m) { (void)d; (void)m; }
static void info_done(void *d, struct zcosmic_toplevel_info_v1 *m)
{ (void)d; (void)m; cosmic_done = 1; }
static void ws_group(void *d, struct ext_workspace_manager_v1 *m,
                     struct ext_workspace_group_handle_v1 *g)
{ (void)d; (void)m; (void)g; }
static void ws_space(void *d, struct ext_workspace_manager_v1 *m,
                     struct ext_workspace_handle_v1 *w)
{ (void)d; (void)m; (void)w; }
static void ws_done(void *d, struct ext_workspace_manager_v1 *m) { (void)d; (void)m; }
static void ws_finished(void *d, struct ext_workspace_manager_v1 *m) { (void)d; (void)m; }
static void reg_remove(void *d, struct wl_registry *r, uint32_t id)
{ (void)d; (void)r; (void)id; }

static void app_id(void *d, struct ext_foreign_toplevel_handle_v1 *h, const char *s)
{ (void)h; set_app(d, s); }
static const struct ext_foreign_toplevel_handle_v1_listener ft_handle_listener = {
    .closed = ft_closed, .done = ft_done, .title = ft_title,
    .app_id = app_id, .identifier = ft_ident
};

static void ft_toplevel(void *data, struct ext_foreign_toplevel_list_v1 *mgr,
                        struct ext_foreign_toplevel_handle_v1 *ft)
{
    (void)data; (void)mgr;
    if (ntops >= MAX_TOPS) return;
    struct top *t = &tops[ntops++];
    memset(t, 0, sizeof(*t));
    t->ft = ft;
    ext_foreign_toplevel_handle_v1_add_listener(ft, &ft_handle_listener, t);
}
static const struct ext_foreign_toplevel_list_v1_listener ft_mgr_listener = {
    .toplevel = ft_toplevel, .finished = ft_finished
};

static void state(void *d, struct zcosmic_toplevel_handle_v1 *h, struct wl_array *a)
{
    struct top *t = d;
    (void)h;
    t->state_count++;
    uint32_t *v = a->data;
    for (size_t i = 0; i < a->size / sizeof(*v); i++) {
        if (v[i] == 0) t->maximized = 1;
        if (v[i] == 1) t->minimized = 1;
        if (v[i] == 2) t->activated = 1;
        if (v[i] == 3) t->fullscreen = 1;
    }
}
static void geometry(void *d, struct zcosmic_toplevel_handle_v1 *h,
                     struct wl_output *o, int32_t x, int32_t y, int32_t w, int32_t hh)
{
    struct top *t = d;
    (void)h; (void)o;
    if (!t->geom_count) { t->x = x; t->y = y; t->w = w; t->h = hh; }
    t->geom_count++;
}
static void workspace_enter_v1(void *d, struct zcosmic_toplevel_handle_v1 *h,
                               struct zcosmic_workspace_handle_v1 *w)
{ (void)d; (void)h; (void)w; }
static const struct zcosmic_toplevel_handle_v1_listener ct_handle_listener = {
    .closed = ct_closed, .done = ct_done, .title = ct_title, .app_id = ct_app,
    .output_enter = ct_output, .output_leave = ct_output,
    .workspace_enter = workspace_enter_v1, .workspace_leave = ct_ws_leave_v1,
    .state = state, .geometry = geometry,
    .ext_workspace_enter = ct_ws_enter_ext, .ext_workspace_leave = ct_ws_leave_ext
};

static void info_toplevel(void *d, struct zcosmic_toplevel_info_v1 *m,
                          struct zcosmic_toplevel_handle_v1 *h)
{
    (void)d; (void)m;
    if (h) zcosmic_toplevel_handle_v1_add_listener(h, &ct_handle_listener, &v1_probe_top);
}
static const struct zcosmic_toplevel_info_v1_listener info_listener = {
    .toplevel = info_toplevel, .finished = info_finished, .done = info_done
};
static const struct ext_workspace_manager_v1_listener ws_listener = {
    .workspace_group = ws_group, .workspace = ws_space,
    .done = ws_done, .finished = ws_finished
};

static void on_global(void *data, struct wl_registry *reg, uint32_t id,
                      const char *iface, uint32_t ver)
{
    (void)data;
    if (iface && !strcmp(iface, "wl_output")) {
        /* Keep the output alive: COSMIC sends geometry only for bound outputs. */
        wl_registry_bind(reg, id, &wl_output_interface, ver < 2 ? ver : 2);
    } else if (iface && !strcmp(iface, "ext_foreign_toplevel_list_v1")) {
        struct ext_foreign_toplevel_list_v1 *m = wl_registry_bind(
            reg, id, &ext_foreign_toplevel_list_v1_interface, 1);
        ext_foreign_toplevel_list_v1_add_listener(m, &ft_mgr_listener, NULL);
        ft_ready = 1;
    } else if (iface && !strcmp(iface, "zcosmic_toplevel_info_v1")) {
        cosmic_mgr = wl_registry_bind(
            reg, id, &zcosmic_toplevel_info_v1_interface, ver < 3 ? ver : 3);
        zcosmic_toplevel_info_v1_add_listener(cosmic_mgr, &info_listener, NULL);
        struct zcosmic_toplevel_info_v1 *v1 = wl_registry_bind(
            reg, id, &zcosmic_toplevel_info_v1_interface, 1);
        zcosmic_toplevel_info_v1_add_listener(v1, &info_listener, NULL);
        cosmic_ready = 1;
    } else if (iface && !strcmp(iface, "ext_workspace_manager_v1")) {
        struct ext_workspace_manager_v1 *m = wl_registry_bind(
            reg, id, &ext_workspace_manager_v1_interface, 1);
        ext_workspace_manager_v1_add_listener(m, &ws_listener, NULL);
    }
}
static const struct wl_registry_listener reg_listener = { .global = on_global, .global_remove = reg_remove };

int main(void)
{
    struct wl_display *d = wl_display_connect(NULL);
    if (!d) { printf("CONNECT_FAIL\n"); return 2; }
    struct wl_registry *r = wl_display_get_registry(d);
    wl_registry_add_listener(r, &reg_listener, NULL);
    wl_display_roundtrip(d);
    wl_display_roundtrip(d);
    if (!cosmic_mgr) { printf("NO_COSMIC_TOPLEVEL_INFO\n"); return 0; }

    for (int i = 0; i < ntops; i++) {
        struct zcosmic_toplevel_handle_v1 *ct =
            zcosmic_toplevel_info_v1_get_cosmic_toplevel(cosmic_mgr, tops[i].ft);
        if (ct) zcosmic_toplevel_handle_v1_add_listener(ct, &ct_handle_listener, &tops[i]);
    }
    for (int round = 0; round < 8; round++) {
        if (wl_display_roundtrip(d) < 0) break;
        if (cosmic_done) break;
    }
    /* COSMIC publishes v2+ properties on its refresh cycle, not on roundtrip. */
    struct pollfd pfd = { .fd = wl_display_get_fd(d), .events = POLLIN };
    for (int i = 0; i < 30; i++) {
        if (poll(&pfd, 1, 100) > 0 && (pfd.revents & POLLIN)) wl_display_dispatch(d);
    }
    printf("READY ft=%d cosmic=%d tops=%d\n", ft_ready, cosmic_ready, ntops);
    for (int i = 0; i < ntops; i++) {
        struct top *t = &tops[i];
        if (t->invalid_app) { printf("INVALID_ROW\n"); continue; }
        printf("ROW|%s|%d|%d|%d|%d|%d|%d|%d|%d\n",
               t->has_app ? t->app : "unknown",
               t->x, t->y, t->w, t->h,
               t->activated, t->minimized, t->maximized, t->fullscreen);
    }
    return 0;
}
