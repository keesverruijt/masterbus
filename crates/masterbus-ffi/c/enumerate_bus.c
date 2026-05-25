/* enumerate_bus — list every device on the bus with its identity and the
 * monitoring groups/fields (with current values and writability).
 *
 * Usage: enumerate_bus <can-iface> [cache-dir]
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "masterbus.h"
#include "mb_print.h"

int main(int argc, char **argv) {
    if (argc < 2) {
        fprintf(stderr, "usage: %s <can-iface> [cache-dir]\n", argv[0]);
        return 1;
    }
    const char *cache = (argc > 2) ? argv[2] : NULL;

    MbBus *bus = mb_open_socketcan(argv[1], cache);
    if (!bus) {
        fprintf(stderr, "connect failed\n");
        return 2;
    }

    uint32_t *ids = NULL;
    int32_t n = mb_devices(bus, &ids);
    printf("%d device(s)\n", n);

    for (int32_t i = 0; i < n; i++) {
        uint32_t id = ids[i];
        char *name = mb_device_name(bus, id);
        char *art = mb_device_article(bus, id);
        char *ser = mb_device_serial(bus, id);
        char *rev = mb_device_revision(bus, id);
        char *fw = mb_device_firmware(bus, id);
        printf("\n=== %s (id %u) art=%s ser=%s rev=%s fw=%s status=%d ===\n",
               name ? name : "?", id, art ? art : "", ser ? ser : "",
               rev ? rev : "", fw ? fw : "", mb_device_status(bus, id));
        mb_free_str(name);
        mb_free_str(art);
        mb_free_str(ser);
        mb_free_str(rev);
        mb_free_str(fw);

        int32_t ng = mb_group_count(bus, id);
        for (int32_t g = 0; g < ng; g++) {
            char *gn = mb_group_name(bus, id, g);
            printf("  [%d] %s\n", g, gn ? gn : "");
            mb_free_str(gn);

            int32_t *fields = NULL;
            int32_t nf = mb_group_fields(bus, id, g, &fields);
            for (int32_t f = 0; f < nf; f++) {
                int32_t fid = fields[f];
                char *fn = mb_field_name(bus, id, fid);
                char *un = mb_field_unit(bus, id, fid);
                const char *wr = mb_field_writable(bus, id, fid) ? "rw" : "ro";
                MbValue *v = mb_field_value(bus, id, fid);
                printf("      %3d %-24s %-4s %s ", fid, fn ? fn : "",
                       un ? un : "", wr);
                print_value(bus, v);
                printf("\n");
                mb_free_value(v);
                mb_free_str(fn);
                mb_free_str(un);
            }
            mb_free_fields(fields, nf);
        }
    }

    mb_free_ids(ids, n);
    mb_close(bus);
    return 0;
}
