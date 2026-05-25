/* Shared value-printing helper for the masterbus C demos. */
#ifndef MB_PRINT_H
#define MB_PRINT_H

#include <stdio.h>
#include "masterbus.h"

/* Print a decoded value in a human-readable form. `bus` is used only to resolve
 * device names for device-reference values; it may be NULL. */
static void print_value(MbBus *bus, const MbValue *v) {
    if (!v) {
        printf("-");
        return;
    }
    switch (mb_value_type(v)) {
    case MbValueType_Float:
        printf("%.3g", mb_value_float(v));
        break;
    case MbValueType_Boolean:
        printf("%s", mb_value_bool(v) ? "true" : "false");
        break;
    case MbValueType_Date: {
        MbDate d = mb_value_date(v);
        printf("%04d-%02d-%02d", d.year, d.mon, d.day);
        break;
    }
    case MbValueType_Time: {
        MbTime t = mb_value_time(v);
        printf("%ud %02d:%02d:%02d", t.days, t.hour, t.min, t.sec);
        break;
    }
    case MbValueType_Text: {
        char *s = mb_value_text(v);
        printf("\"%s\"", s ? s : "");
        mb_free_str(s);
        break;
    }
    case MbValueType_List:
    case MbValueType_Eventable: {
        int32_t idx = mb_value_list_index(v);
        char *lbl = mb_value_list_label(v, idx);
        printf("[%d] %s", idx, lbl ? lbl : "");
        mb_free_str(lbl);
        break;
    }
    case MbValueType_DeviceRef: {
        int32_t idx = mb_value_list_index(v);
        uint32_t did = mb_value_device_id(v, idx);
        char *dn = did ? mb_device_name(bus, did) : NULL;
        printf("-> %u %s", did, dn ? dn : "");
        mb_free_str(dn);
        break;
    }
    default:
        printf("invalid");
    }
}

#endif /* MB_PRINT_H */
