package bluemaprs.paper.players;

import bluemaprs.paper.ipc.CoreLink;
import bluemaprs.paper.ipc.Frame;
import bluemaprs.paper.ipc.Msg;
import bluemaprs.paper.ipc.Proto.PlayerInfo;
import com.destroystokyo.paper.event.player.PlayerPostRespawnEvent;
import com.google.gson.JsonObject;
import de.bluecolored.bluemap.api.plugin.PlayerDisplayNameProvider;
import io.papermc.paper.threadedregions.scheduler.ScheduledTask;
import org.bukkit.Bukkit;
import org.bukkit.entity.Player;
import org.bukkit.event.EventHandler;
import org.bukkit.event.EventPriority;
import org.bukkit.event.Listener;
import org.bukkit.event.player.PlayerJoinEvent;
import org.bukkit.event.player.PlayerQuitEvent;
import org.bukkit.plugin.Plugin;

import java.io.IOException;
import java.util.ArrayList;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.Executors;
import java.util.concurrent.ScheduledExecutorService;
import java.util.concurrent.TimeUnit;
import java.util.function.Supplier;
import java.util.logging.Level;
import java.util.logging.Logger;

/**
 * Online players as upstream's {@code BukkitPlugin} tracks them: snapshots every 20 ticks on the entity scheduler
 * (Paper and Folia), sent as one {@code Players} batch per second. The core applies every hide rule.
 */
public final class PlayerTracker implements Listener {

    private final Plugin plugin;
    private final CoreLink link;
    private final Supplier<PlayerDisplayNameProvider> displayNames;
    private final Logger log;
    /** Iteration order of this map is upstream's live-players JSON order. */
    private final Map<UUID, Tracked> online = new ConcurrentHashMap<>();
    private final ScheduledExecutorService sender = Executors.newSingleThreadScheduledExecutor(r -> {
        Thread thread = new Thread(r, "BlueMap-Players");
        thread.setDaemon(true);
        return thread;
    });

    private static final class Tracked {
        volatile PlayerSnapshot snapshot;
        volatile ScheduledTask task;

        Tracked(PlayerSnapshot snapshot) {
            this.snapshot = snapshot;
        }
    }

    public PlayerTracker(Plugin plugin, CoreLink link, Supplier<PlayerDisplayNameProvider> displayNames, Logger log) {
        this.plugin = plugin;
        this.link = link;
        this.displayNames = displayNames;
        this.log = log;
    }

    public void start() {
        for (Player player : Bukkit.getOnlinePlayers()) track(player);
        sender.scheduleAtFixedRate(this::sendBatch, 1, 1, TimeUnit.SECONDS);
    }

    public void stop() {
        sender.shutdownNow();
        online.values().forEach(PlayerTracker::cancel);
        online.clear();
    }

    /** Upstream's default {@code PlayerDisplayNameProvider}. */
    public String accountName(UUID uuid) {
        Tracked tracked = online.get(uuid);
        return tracked != null ? tracked.snapshot.accountName() : uuid.toString();
    }

    /** After a core restart: send the current batch without waiting for the next tick. */
    public void resend() {
        sender.execute(this::sendBatch);
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onPlayerJoin(PlayerJoinEvent event) {
        track(event.getPlayer());
        sendUuid("PlayerJoin", event.getPlayer().getUniqueId());
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onPlayerLeave(PlayerQuitEvent event) {
        UUID uuid = event.getPlayer().getUniqueId();
        Tracked tracked = online.remove(uuid);
        if (tracked != null) cancel(tracked);
        sendUuid("PlayerLeave", uuid);
    }

    @EventHandler(priority = EventPriority.MONITOR, ignoreCancelled = true)
    public void onPlayerRespawn(PlayerPostRespawnEvent event) {
        track(event.getPlayer());
    }

    private void track(Player player) {
        Tracked tracked = new Tracked(PlayerSnapshot.of(player));
        Tracked previous = online.put(player.getUniqueId(), tracked);
        if (previous != null) cancel(previous);
        tracked.task = player.getScheduler().runAtFixedRate(plugin,
                task -> tracked.snapshot = PlayerSnapshot.of(player), null, 20, 20);
    }

    private void sendBatch() {
        if (!link.connected()) return;
        try {
            PlayerDisplayNameProvider names = displayNames.get();
            List<PlayerInfo> players = new ArrayList<>(online.size());
            for (Tracked tracked : online.values()) {
                PlayerSnapshot snapshot = tracked.snapshot;
                players.add(snapshot.toInfo(displayName(names, snapshot)));
            }
            JsonObject header = Msg.of("Players");
            header.add("players", Msg.GSON.toJsonTree(players));
            link.sendLatest("players", List.of(new Frame(header)));
        } catch (RuntimeException e) {
            log.log(Level.WARNING, "Failed to send the online players to the BlueMap core", e);
        }
    }

    private String displayName(PlayerDisplayNameProvider names, PlayerSnapshot snapshot) {
        try {
            String name = names.get(snapshot.uuid());
            return name != null ? name : snapshot.accountName();
        } catch (RuntimeException e) {
            log.log(Level.FINE, "PlayerDisplayNameProvider failed", e);
            return snapshot.accountName();
        }
    }

    private void sendUuid(String type, UUID uuid) {
        if (!link.connected()) return;
        JsonObject header = Msg.of(type);
        header.addProperty("uuid", uuid.toString());
        try {
            link.send(new Frame(header));
        } catch (IOException e) {
            log.fine(type + " not sent: " + e.getMessage());
        }
    }

    private static void cancel(Tracked tracked) {
        ScheduledTask task = tracked.task;
        if (task != null) task.cancel();
    }

}
