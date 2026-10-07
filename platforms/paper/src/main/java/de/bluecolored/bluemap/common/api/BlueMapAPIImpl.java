package de.bluecolored.bluemap.common.api;

import bluemaprs.paper.api.MarkerStore;
import bluemaprs.paper.api.Mirror;
import bluemaprs.paper.api.ShimBackend;
import com.flowpowered.math.vector.Vector2i;
import de.bluecolored.bluemap.api.BlueMapAPI;
import de.bluecolored.bluemap.api.BlueMapMap;
import de.bluecolored.bluemap.api.BlueMapWorld;
import de.bluecolored.bluemap.common.BlueMapService;
import de.bluecolored.bluemap.core.logger.Logger;

import java.util.Collection;
import java.util.Map;
import java.util.Objects;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Predicate;
import java.util.stream.Collectors;

/** Upstream's class name so casts succeed; backed by the core's mirror, see docs/13 §3. */
public class BlueMapAPIImpl extends BlueMapAPI {

    public static final String BLUEMAP_VERSION = "5.28";

    static final String NOT_AVAILABLE = " is not available in bluemap-rs; the core runs out of process";

    private final ShimBackend backend;
    private final MarkerStore markers;
    private final WebAppImpl webApp;
    private final RenderManagerImpl renderManager;
    private final PluginImpl plugin;
    private final Map<String, Predicate<Vector2i>> tileFilters = new ConcurrentHashMap<>();

    public BlueMapAPIImpl(ShimBackend backend, MarkerStore markers) {
        this.backend = backend;
        this.markers = markers;
        this.webApp = new WebAppImpl(backend);
        this.renderManager = new RenderManagerImpl(backend);
        this.plugin = new PluginImpl(backend.providers());
    }

    @Override
    public Collection<BlueMapMap> getMaps() {
        return backend.mirror().maps().stream()
                .map(map -> getMap(map.id()))
                .flatMap(Optional::stream)
                .collect(Collectors.toUnmodifiableSet());
    }

    @Override
    public Collection<BlueMapWorld> getWorlds() {
        return backend.mirror().worlds().stream()
                .map(world -> (BlueMapWorld) new BlueMapWorldImpl(this, world.id()))
                .collect(Collectors.toUnmodifiableSet());
    }

    @Override
    public Optional<BlueMapWorld> getWorld(Object world) {
        Objects.requireNonNull(world, "world");
        Mirror mirror = backend.mirror();
        if (world instanceof String id && mirror.world(id).isPresent())
            return Optional.of(new BlueMapWorldImpl(this, id));

        return backend.serverWorldId(world)
                .flatMap(mirror::worldOfServerWorld)
                .map(coreWorld -> new BlueMapWorldImpl(this, coreWorld.id()));
    }

    @Override
    public Optional<BlueMapMap> getMap(String id) {
        Objects.requireNonNull(id, "id");
        return backend.mirror().map(id)
                .filter(map -> backend.mirror().world(map.world()).isPresent())
                .map(map -> new BlueMapMapImpl(this, map.id(), new BlueMapWorldImpl(this, map.world())));
    }

    @Override
    public String getBlueMapVersion() {
        return BLUEMAP_VERSION;
    }

    @Override
    public WebAppImpl getWebApp() {
        return webApp;
    }

    @Override
    public RenderManagerImpl getRenderManager() {
        return renderManager;
    }

    @Override
    public de.bluecolored.bluemap.api.plugin.Plugin getPlugin() {
        return plugin;
    }

    public void register() {
        try {
            BlueMapAPI.registerInstance(this);
        } catch (Exception ex) {
            Logger.global.logError("BlueMapAPI: A BlueMapAPI listener threw an exception (onEnable)!", ex);
        }
    }

    public void unregister() {
        try {
            BlueMapAPI.unregisterInstance(this);
        } catch (Exception ex) {
            Logger.global.logError("BlueMapAPI: A BlueMapAPI listener threw an exception (onDisable)!", ex);
        }
    }

    /** Upstream unwrap for addons depending on BlueMapCommon. */
    public BlueMapService blueMapService() {
        throw new UnsupportedOperationException("BlueMapAPIImpl.blueMapService()" + NOT_AVAILABLE);
    }

    /** Upstream unwrap for addons depending on BlueMapCommon. */
    public de.bluecolored.bluemap.common.plugin.Plugin plugin() {
        throw new UnsupportedOperationException("BlueMapAPIImpl.plugin()" + NOT_AVAILABLE);
    }

    ShimBackend backend() {
        return backend;
    }

    MarkerStore markers() {
        return markers;
    }

    Map<String, Predicate<Vector2i>> tileFilters() {
        return tileFilters;
    }

}
