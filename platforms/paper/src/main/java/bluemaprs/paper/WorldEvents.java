package bluemaprs.paper;

import bluemaprs.paper.ipc.CoreLink;
import bluemaprs.paper.ipc.Frame;
import bluemaprs.paper.ipc.Msg;
import com.google.gson.JsonObject;
import org.bukkit.Bukkit;
import org.bukkit.World;
import org.bukkit.event.EventHandler;
import org.bukkit.event.EventPriority;
import org.bukkit.event.Listener;
import org.bukkit.event.world.WorldLoadEvent;
import org.bukkit.event.world.WorldUnloadEvent;
import org.bukkit.plugin.Plugin;

import java.io.IOException;
import java.util.concurrent.CompletableFuture;
import java.util.logging.Logger;

/** World load/unload (Multiverse) → {@code WorldAdded/Removed}; core-requested saves ({@code SaveWorld}). */
final class WorldEvents implements Listener {

    private final Plugin plugin;
    private final CoreLink link;
    private final Logger log;
    private volatile boolean stopping;

    WorldEvents(Plugin plugin, CoreLink link, Logger log) {
        this.plugin = plugin;
        this.link = link;
        this.log = log;
    }

    void stopping() {
        stopping = true;
    }

    @EventHandler(priority = EventPriority.MONITOR)
    public void onWorldLoad(WorldLoadEvent event) {
        JsonObject header = Msg.of("WorldAdded");
        header.add("world", Msg.GSON.toJsonTree(ServerInfo.worldInfo(event.getWorld())));
        send(header);
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onWorldUnload(WorldUnloadEvent event) {
        JsonObject header = Msg.of("WorldRemoved");
        header.addProperty("id", event.getWorld().getKey().toString());
        send(header);
    }

    /** Upstream {@code BukkitWorld.persistWorldChanges}: main-thread {@code world.save()}; never on Folia. */
    CompletableFuture<Boolean> save(String worldId) {
        // the main thread is blocked in onDisable while the core shuts down
        if (ServerInfo.IS_FOLIA || stopping) return CompletableFuture.completedFuture(false);
        return CompletableFuture.supplyAsync(() -> {
            if (worldId == null) {
                Bukkit.getWorlds().forEach(World::save);
                return true;
            }
            World world = ServerInfo.worldById(worldId);
            if (world == null) return false;
            world.save();
            return true;
        }, Bukkit.getScheduler().getMainThreadExecutor(plugin));
    }

    private void send(JsonObject header) {
        if (!link.connected()) return; // the next Hello carries the current world list
        try {
            link.send(new Frame(header));
        } catch (IOException e) {
            log.fine(Msg.string(header, "t") + " not sent: " + e.getMessage());
        }
    }

}
