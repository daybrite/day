// Copyright © The Daybrite Project
// SPDX-License-Identifier: MPL-2.0

#include <Network/Network.h>
#include <dispatch/dispatch.h>
#include <stdatomic.h>

// The three bits are published together from the serial monitor queue. Keep the
// monitor for the process lifetime so reads on the UI thread never wait for a path.
unsigned day_connectivity_path_flags(void) {
    static dispatch_once_t once;
    static nw_path_monitor_t monitor;
    static atomic_uint flags;
    dispatch_once(&once, ^{
        monitor = nw_path_monitor_create();
        if (!monitor) return;
        nw_path_monitor_set_queue(monitor, dispatch_queue_create("dev.daybrite.connectivity", NULL));
        nw_path_monitor_set_update_handler(monitor, ^(nw_path_t path) {
            unsigned value = 0;
            if (nw_path_get_status(path) == nw_path_status_satisfied) {
                value = 1 | (nw_path_is_expensive(path) ? 2 : 0)
                          | (nw_path_is_constrained(path) ? 4 : 0);
            }
            atomic_store(&flags, value);
        });
        nw_path_monitor_start(monitor);
    });
    return atomic_load(&flags);
}
