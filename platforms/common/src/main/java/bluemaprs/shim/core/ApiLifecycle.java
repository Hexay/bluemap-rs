package bluemaprs.shim.core;

import bluemaprs.shim.api.MarkerStore;
import bluemaprs.shim.api.Mirror;
import bluemaprs.shim.api.ShimBackend;
import bluemaprs.shim.ipc.Proto.MapInfo;
import bluemaprs.shim.ipc.Proto.NotReady;
import bluemaprs.shim.ipc.Proto.ReadyInfo;
import bluemaprs.shim.markers.MarkerPusher;
import bluemaprs.shim.players.PlayerRegistry;
import com.google.gson.JsonParseException;
import com.google.gson.reflect.TypeToken;
import de.bluecolored.bluemap.api.gson.MarkerGson;
import de.bluecolored.bluemap.api.markers.MarkerSet;
import de.bluecolored.bluemap.common.api.BlueMapAPIImpl;
import org.slf4j.Logger;

import java.lang.reflect.Type;
import java.util.Map;
import java.util.function.Consumer;

/**
 * BlueMapAPI registration across core loads (docs/13 §3): a fresh API instance per load, kept registered across core
 * crashes. Single-threaded: the {@code BlueMap-Load} thread, or the main thread after it stopped.
 */
final class ApiLifecycle {

    private static final Type MARKER_SETS = new TypeToken<Map<String, MarkerSet>>() {}.getType();

    private final Logger log;
    private final ShimBackend backend;
    private final MarkerPusher markers;
    private final PlayerRegistry players;
    private final Consumer<ReadyInfo> onReady;
    private volatile BlueMapAPIImpl api;

    ApiLifecycle(Logger log, ShimBackend backend, MarkerPusher markers, PlayerRegistry players,
                 Consumer<ReadyInfo> onReady) {
        this.log = log;
        this.backend = backend;
        this.markers = markers;
        this.players = players;
        this.onReady = onReady;
    }

    /**
     * @param reconnect the first {@code Ready} of a respawned core
     * @param forceCycle re-register even if the map set is unchanged (manual {@code /bluemap reload})
     */
    void ready(ReadyInfo ready, boolean reconnect, boolean forceCycle) {
        Mirror previous = backend.mirror();
        backend.setReady(ready);
        if (api != null) {
            if (reconnect && !forceCycle && previous.mapIds().equals(backend.mirror().mapIds())) {
                log.info("Reconnected to the BlueMap core; replaying markers, scripts and players");
                markers.replayAll();
                backend.replayAll();
                players.resend();
                return;
            }
            unregister();
        }
        register(ready);
        backend.replayPending();
        if (reconnect) players.resend();
        onReady.accept(ready);
    }

    void notReady(NotReady notReady) {
        String message = "BlueMap could not load (" + notReady.reason() + "): " + notReady.message()
                + " Fix the cause and run /bluemap reload.";
        if ("no-maps".equals(notReady.reason())) log.warn(message);
        else log.error(message);
        unregister();
    }

    /** {@code Unloading}, a load failure or server stop; {@code onDisable} consumers still see a working API. */
    void unregister() {
        BlueMapAPIImpl current = api;
        if (current == null) return;
        api = null;
        current.unregister();
        backend.clearWebAssets();
    }

    private void register(ReadyInfo ready) {
        MarkerStore store = new MarkerStore();
        for (MapInfo map : ready.maps()) loadConfigMarkers(map, store.sets(map.id()));
        markers.setStore(store);

        BlueMapAPIImpl fresh = new BlueMapAPIImpl(backend, store);
        api = fresh;
        fresh.register();
        markers.replayAll();
    }

    private void loadConfigMarkers(MapInfo map, Map<String, MarkerSet> sets) {
        String json = map.configMarkers();
        if (json == null || json.isBlank()) return;
        try {
            Map<String, MarkerSet> parsed = MarkerGson.INSTANCE.fromJson(json, MARKER_SETS);
            if (parsed != null) sets.putAll(parsed);
        } catch (JsonParseException e) {
            log.warn("Failed to load the configured marker-sets of map '" + map.id() + "': " + e.getMessage());
        }
    }

}
