package bluemaprs.shim.skins;

import bluemaprs.shim.api.ShimBackend;
import bluemaprs.shim.ipc.Msg;
import bluemaprs.shim.ipc.Proto.MapInfo;
import bluemaprs.shim.ipc.Proto.ReadyInfo;
import com.google.gson.JsonObject;
import de.bluecolored.bluemap.core.logger.Logger;

import javax.imageio.ImageIO;
import java.awt.image.BufferedImage;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.util.List;
import java.util.Map;
import java.util.Optional;
import java.util.UUID;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.CompletionException;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ExecutorService;
import java.util.concurrent.Executors;
import java.util.concurrent.TimeUnit;

/**
 * Upstream's {@code common.plugin.skins.PlayerSkinUpdater}: on join (if {@code skin-download} is on), at most once an
 * hour per player, loads the skin with the API's {@link de.bluecolored.bluemap.api.plugin.SkinProvider}, renders it
 * with the {@link de.bluecolored.bluemap.api.plugin.PlayerIconFactory} and writes {@code playerheads/<uuid>.png} to
 * every loaded map's storage; an empty skin deletes that asset.
 */
public final class PlayerSkinUpdater {

    private final ShimBackend backend;
    private final Map<UUID, Long> skinUpdates = new ConcurrentHashMap<>();
    private final ExecutorService pool = Executors.newFixedThreadPool(2, r -> {
        Thread thread = new Thread(r, "BlueMap-Skins");
        thread.setDaemon(true);
        return thread;
    });

    public PlayerSkinUpdater(ShimBackend backend) {
        this.backend = backend;
    }

    /** A new core load starts a new updater upstream, which forgets the hourly limits. */
    public void reset() {
        skinUpdates.clear();
    }

    public void stop() {
        pool.shutdownNow();
    }

    public void onPlayerJoin(UUID uuid) {
        ReadyInfo ready = backend.mirror().ready();
        if (ready == null || ready.plugin() == null || !ready.plugin().skinDownload()) return;
        updateSkin(uuid).exceptionally(ex -> {
            Logger.global.logError("Failed to update player skin: " + uuid, ex);
            return null;
        });
    }

    public CompletableFuture<Void> updateSkin(UUID uuid) {
        long now = System.currentTimeMillis();
        long last = skinUpdates.getOrDefault(uuid, 0L);
        if (now - last < TimeUnit.HOURS.toMillis(1)) return CompletableFuture.completedFuture(null);
        skinUpdates.put(uuid, now);

        return CompletableFuture.supplyAsync(() -> {
            try {
                return backend.providers().getSkinProvider().load(uuid);
            } catch (IOException e) {
                throw new CompletionException("The skin provider threw an exception while loading the skin for UUID: '" + uuid + "'!", e);
            }
        }, pool).thenAcceptAsync(skin -> write(uuid, skin), pool);
    }

    private void write(UUID uuid, Optional<BufferedImage> skin) {
        List<MapInfo> maps = backend.mirror().maps();
        if (backend.mirror().ready() == null) {
            Logger.global.logDebug("Could not update skin, since the plugin seems not to be ready.");
            return;
        }
        String name = "playerheads/" + uuid + ".png";
        if (skin.isEmpty()) {
            Logger.global.logDebug("Empty player-skin provided for UUID: " + uuid);
            for (MapInfo map : maps) {
                try {
                    backend.call(asset("AssetDelete", map.id(), name));
                } catch (IOException ex) {
                    Logger.global.logError("Failed to remove player skin from storage: " + uuid, ex);
                }
            }
            return;
        }

        BufferedImage head = backend.providers().getPlayerMarkerIconFactory().apply(uuid, skin.get());
        byte[] png;
        try {
            ByteArrayOutputStream out = new ByteArrayOutputStream();
            ImageIO.write(head, "png", out);
            png = out.toByteArray();
        } catch (IOException ex) {
            Logger.global.logError("Failed to write player skin to storage: " + uuid, ex);
            return;
        }
        for (MapInfo map : maps) {
            try {
                backend.call(asset("AssetWrite", map.id(), name), png);
            } catch (IOException ex) {
                Logger.global.logError("Failed to write player skin to storage: " + uuid, ex);
            }
        }
    }

    private static JsonObject asset(String type, String map, String name) {
        JsonObject header = Msg.of(type);
        header.addProperty("map", map);
        header.addProperty("name", name);
        return header;
    }

}
