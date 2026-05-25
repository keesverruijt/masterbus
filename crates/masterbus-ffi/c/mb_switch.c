/* mb_switch — write a value to a field, dispatching by the field's type.
 *
 * Reads the field first to learn its type, then writes a boolean or a float
 * accordingly and reports the value observed afterwards.
 *
 * Usage: mb_switch <can-iface> <device-id> <field> <value>
 *   <value> for a boolean: on/off, true/false, 1/0
 *   <value> for a float:   a decimal number
 */
#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <strings.h>

#include "masterbus.h"
#include "mb_print.h"

static bool parse_bool(const char *s) {
    return strcmp(s, "1") == 0 || strcasecmp(s, "on") == 0 ||
           strcasecmp(s, "true") == 0;
}

int main(int argc, char **argv) {
    if (argc < 5) {
        fprintf(stderr, "usage: %s <can-iface> <device-id> <field> <value>\n",
                argv[0]);
        return 1;
    }
    MbBus *bus = mb_open_socketcan(argv[1], NULL);
    if (!bus) {
        fprintf(stderr, "connect failed\n");
        return 2;
    }
    uint32_t id = (uint32_t)strtoul(argv[2], NULL, 0);
    int32_t field = (int32_t)strtol(argv[3], NULL, 0);
    const char *arg = argv[4];

    if (!mb_field_writable(bus, id, field)) {
        fprintf(stderr, "field %d is not writable\n", field);
        mb_close(bus);
        return 3;
    }

    MbValue *cur = mb_field_value(bus, id, field);
    if (!cur) {
        fprintf(stderr, "cannot read field %d\n", field);
        mb_close(bus);
        return 4;
    }
    MbValueType ty = mb_value_type(cur);
    printf("before: ");
    print_value(bus, cur);
    printf("\n");
    mb_free_value(cur);

    MbValue *res = NULL;
    if (ty == MbValueType_Boolean) {
        res = mb_set_bool(bus, id, field, parse_bool(arg));
    } else if (ty == MbValueType_Float) {
        res = mb_set_float(bus, id, field, (float)atof(arg));
    } else {
        fprintf(stderr, "field type %d is not settable by this demo\n", ty);
        mb_close(bus);
        return 5;
    }

    if (!res) {
        fprintf(stderr, "write failed or was rejected by the device\n");
        mb_close(bus);
        return 6;
    }
    printf("after:  ");
    print_value(bus, res);
    printf("\n");
    mb_free_value(res);
    mb_close(bus);
    return 0;
}
