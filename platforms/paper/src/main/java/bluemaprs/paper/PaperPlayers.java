package bluemaprs.paper;

import bluemaprs.shim.players.PlayerRegistry;
import bluemaprs.shim.players.PlayerSnapshot;
import com.destroystokyo.paper.event.player.PlayerPostRespawnEvent;
import io.papermc.paper.threadedregions.scheduler.ScheduledTask;
import org.bukkit.Bukkit;
import org.bukkit.GameMode;
import org.bukkit.Location;
import org.bukkit.block.Block;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.EventPriority;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.event.player.PlayerQuitEvent;
import org.bukkit.plugin.Plugin;
import org.bukkit.potion.PotionEffectType;
import org.bukkit.metadata.MetadataValue;

import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/** Snapshots every 20 ticks on each player's entity scheduler (Paper and Folia), as upstream's {@code BukkitPlugin}. */
final class PaperPlayers implements Listener {

    private final Plugin plugin;
    private final PlayerRegistry registry;
    private final Map<UUID, ScheduledTask> tasks = new ConcurrentHashMap<>();

    PaperPlayers(Plugin plugin, PlayerRegistry registry) {
        this.plugin = plugin;
        this.registry = registry;
    }

    void start() {
        for (Player player : Bukkit.getOnlinePlayers()) {
            registry.track(snapshot(player));
            schedule(player);
        }
    }

    void stop() {
        tasks.values().forEach(ScheduledTask::cancel);
        tasks.clear();
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onPlayerJoin(PlayerJoinEvent event) {
        registry.join(snapshot(event.getPlayer()));
        schedule(event.getPlayer());
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onPlayerLeave(PlayerQuitEvent event) {
        UUID uuid = event.getPlayer().getUniqueId();
        ScheduledTask task = tasks.remove(uuid);
        if (task != null) task.cancel();
        registry.leave(uuid);
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onPlayerRespawn(PlayerPostRespawnEvent event) {
        registry.track(snapshot(event.getPlayer()));
        schedule(event.getPlayer());
    }

    private void schedule(Player player) {
        ScheduledTask task = player.getScheduler().runAtFixedRate(plugin,
                t -> registry.update(snapshot(player)), null, 20, 20);
        ScheduledTask previous = task == null ? null : tasks.put(player.getUniqueId(), task);
        if (previous != null) previous.cancel();
    }

    /** Upstream {@code BukkitPlayer.update}; must be taken on the player's (entity scheduler) thread. */
    private static PlayerSnapshot snapshot(Player player) {
        boolean vanished = false;
        for (MetadataValue meta : player.getMetadata("vanished")) {
            if (meta.asBoolean()) vanished = true;
        }

        Location location = player.getLocation();
        Block block = location.getBlock();
        return new PlayerSnapshot(
                player.getUniqueId(),
                player.getName(),
                player.getWorld().getKey().toString(),
                location.getX(), location.getY(), location.getZ(),
                // widened floats, as upstream's Vector3d(pitch, yaw, 0)
                (double) location.getPitch(), (double) location.getYaw(),
                block.getLightFromSky(), block.getLightFromBlocks(),
                player.isSneaking(),
                player.hasPotionEffect(PotionEffectType.INVISIBILITY),
                vanished,
                gamemode(player.getGameMode())
        );
    }

    // equality, not a switch: GameMode may stop being an enum on newer API versions
    private static String gamemode(GameMode mode) {
        if (mode == GameMode.CREATIVE) return "creative";
        if (mode == GameMode.ADVENTURE) return "adventure";
        if (mode == GameMode.SPECTATOR) return "spectator";
        return "survival";
    }

}
