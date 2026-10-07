package de.bluecolored.bluemap.common.api;

import bluemaprs.paper.ipc.Proto.MapInfo;
import com.flowpowered.math.vector.Vector2i;
import de.bluecolored.bluemap.api.AssetStorage;
import de.bluecolored.bluemap.api.BlueMapMap;
import de.bluecolored.bluemap.api.BlueMapWorld;
import de.bluecolored.bluemap.api.markers.MarkerSet;
import de.bluecolored.bluemap.core.logger.Logger;

import java.util.Map;
import java.util.Objects;
import java.util.concurrent.atomic.AtomicBoolean;
import java.util.function.Predicate;

public class BlueMapMapImpl implements BlueMapMap {

    private static final AtomicBoolean TILE_FILTER_WARNED = new AtomicBoolean();

    private final BlueMapAPIImpl api;
    private final String mapId;
    private final BlueMapWorldImpl world;

    BlueMapMapImpl(BlueMapAPIImpl api, String mapId, BlueMapWorldImpl world) {
        this.api = api;
        this.mapId = mapId;
        this.world = world;
    }

    @Override
    public String getId() {
        return mapId;
    }

    @Override
    public String getName() {
        return info().name();
    }

    @Override
    public BlueMapWorld getWorld() {
        return world;
    }

    @Override
    public AssetStorage getAssetStorage() {
        return new AssetStorageImpl(api.backend(), mapId);
    }

    @Override
    public Map<String, MarkerSet> getMarkerSets() {
        return api.markers().sets(mapId);
    }

    @Override
    public Vector2i getTileSize() {
        int[] size = info().tileSize();
        return new Vector2i(size[0], size[1]);
    }

    @Override
    public Vector2i getTileOffset() {
        int[] offset = info().tileOffset();
        return new Vector2i(offset[0], offset[1]);
    }

    @Override
    @SuppressWarnings("removal")
    @Deprecated(forRemoval = true)
    public void setTileFilter(Predicate<Vector2i> filter) {
        Objects.requireNonNull(filter, "filter");
        if (TILE_FILTER_WARNED.compareAndSet(false, true))
            Logger.global.logWarning("A plugin called the deprecated BlueMapMap.setTileFilter, which bluemap-rs "
                    + "ignores; use the map's render-mask config instead.");
        api.tileFilters().put(mapId, filter);
    }

    @Override
    @SuppressWarnings("removal")
    @Deprecated(forRemoval = true)
    public Predicate<Vector2i> getTileFilter() {
        return api.tileFilters().getOrDefault(mapId, tile -> true);
    }

    @Override
    public void setFrozen(boolean frozen) {
        if (frozen != isFrozen()) api.backend().setFrozen(mapId, frozen);
    }

    @Override
    public boolean isFrozen() {
        return api.backend().isFrozen(mapId);
    }

    @Override
    public boolean equals(Object o) {
        return this == o || o instanceof BlueMapMapImpl that && mapId.equals(that.mapId);
    }

    @Override
    public int hashCode() {
        return mapId.hashCode();
    }

    /** Upstream unwrap for addons depending on BlueMapCore. */
    public de.bluecolored.bluemap.core.map.BmMap map() {
        throw new UnsupportedOperationException("BlueMapMapImpl.map()" + BlueMapAPIImpl.NOT_AVAILABLE);
    }

    private MapInfo info() {
        return api.backend().mirror().map(mapId).orElseThrow(() -> new IllegalStateException(
                "Map '" + mapId + "' is no longer loaded. Most likely BlueMap got reloaded and this instance is no longer valid."));
    }

}
