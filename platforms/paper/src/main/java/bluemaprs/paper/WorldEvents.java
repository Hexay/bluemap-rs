package bluemaprs.paper;

import bluemaprs.shim.ShimCore;
import org.bukkit.event.EventHandler;
import org.bukkit.event.EventPriority;
import org.bukkit.event.Listener;
import org.bukkit.event.world.WorldLoadEvent;
import org.bukkit.event.world.WorldUnloadEvent;

/** World load/unload (Multiverse) → {@code WorldAdded/Removed}. */
final class WorldEvents implements Listener {

    private final ShimCore<?> core;

    WorldEvents(ShimCore<?> core) {
        this.core = core;
    }

    @EventHandler(priority = EventPriority.MONITOR)
    public void onWorldLoad(WorldLoadEvent event) {
        core.worldAdded(PaperPlatform.worldInfo(event.getWorld()));
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onWorldUnload(WorldUnloadEvent event) {
        core.worldRemoved(event.getWorld().getKey().toString());
    }

}
