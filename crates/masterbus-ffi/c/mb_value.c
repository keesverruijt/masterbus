/* mb_value — read and print one field's current value.
 *
 * Usage: mb_value <can-iface> <device-id> <field>
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>

#include "masterbus.h"
#include "mb_print.h"

int main(int argc, char **argv) {
    if (argc < 4) {
        fprintf(stderr, "usage: %s <can-iface> <device-id> <field>\n", argv[0]);
        return 1;
    }
    MbBus *bus = mb_open_socketcan(argv[1], NULL);
    if (!bus) {
        fprintf(stderr, "connect failed\n");
        return 2;
    }
    uint32_t id = (uint32_t)strtoul(argv[2], NULL, 0);
    int32_t field = (int32_t)strtol(argv[3], NULL, 0);

    char *fn = mb_field_name(bus, id, field);
    char *un = mb_field_unit(bus, id, field);
    MbValue *v = mb_field_value(bus, id, field);

    printf("%s [%d] = ", fn ? fn : "?", field);
    print_value(bus, v);
    if (un && un[0]) {
        printf(" %s", un);
    }
    printf("\n");

    int rc = v ? 0 : 3;
    mb_free_value(v);
    mb_free_str(fn);
    mb_free_str(un);
    mb_close(bus);
    return rc;
}
