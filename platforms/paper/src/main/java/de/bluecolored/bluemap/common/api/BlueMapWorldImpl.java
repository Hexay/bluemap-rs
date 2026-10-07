package de.bluecolored.bluemap.common.api;

import de.bluecolored.bluemap.api.BlueMapMap;
import de.bluecolored.bluemap.api.BlueMapWorld;

import java.nio.file.Path;
import java.util.Collection;
import java.util.stream.Collectors;

public class BlueMapWorldImpl implements BlueMapWorld {

    private final BlueMapAPIImpl api;
    private final String id;

    BlueMapWorldImpl(BlueMapAPIImpl api, String id) {
        this.api = api;
        this.id = id;
    }

    @Override
    public String getId() {
        return id;
    }

    @Override
    @Deprecated
    public Path getSaveFolder() {
        return api.backend().mirror().world(id)
                .filter(world -> world.saveFolder() != null)
                .map(world -> Path.of(world.saveFolder()))
                .orElseThrow(() -> new UnsupportedOperationException("This world-type has no save-folder."));
    }

    @Override
    public Collection<BlueMapMap> getMaps() {
        return api.backend().mirror().maps().stream()
                .filter(map -> map.world().equals(id))
                .map(map -> (BlueMapMap) new BlueMapMapImpl(api, map.id(), this))
                .collect(Collectors.toUnmodifiableSet());
    }

    @Override
    public boolean equals(Object o) {
        return this == o || o instanceof BlueMapWorldImpl that && id.equals(that.id);
    }

    @Override
    public int hashCode() {
        return id.hashCode();
    }

    /** Upstream unwrap for addons depending on BlueMapCore. */
    public de.bluecolored.bluemap.core.world.World world() {
        throw new UnsupportedOperationException("BlueMapWorldImpl.world()" + BlueMapAPIImpl.NOT_AVAILABLE);
    }

}
