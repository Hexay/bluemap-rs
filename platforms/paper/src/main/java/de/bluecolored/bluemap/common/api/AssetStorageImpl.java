package de.bluecolored.bluemap.common.api;

import bluemaprs.paper.api.ShimBackend;
import bluemaprs.paper.ipc.Msg;
import bluemaprs.paper.ipc.Reply;
import com.google.gson.JsonObject;
import de.bluecolored.bluemap.api.AssetStorage;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.Objects;
import java.util.Optional;

/** Assets are KB-sized icons: writes buffer in memory and go to the core as one {@code AssetWrite} on close. */
public class AssetStorageImpl implements AssetStorage {

    private final ShimBackend backend;
    private final String mapId;

    AssetStorageImpl(ShimBackend backend, String mapId) {
        this.backend = backend;
        this.mapId = mapId;
    }

    @Override
    public OutputStream writeAsset(String name) {
        JsonObject header = header("AssetWrite", name);
        return new ByteArrayOutputStream() {
            private boolean closed;

            @Override
            public void close() throws IOException {
                if (closed) return;
                closed = true;
                backend.call(header, toByteArray());
            }
        };
    }

    @Override
    public Optional<InputStream> readAsset(String name) throws IOException {
        Reply reply = backend.call(header("AssetRead", name));
        if (!reply.bool(false)) return Optional.empty();
        return Optional.of(new ByteArrayInputStream(reply.body()));
    }

    @Override
    public boolean assetExists(String name) throws IOException {
        return backend.call(header("AssetExists", name)).bool(false);
    }

    @Override
    public String getAssetUrl(String name) {
        return "maps/" + mapId + "/assets/" + escapeAssetName(Objects.requireNonNull(name, "name"));
    }

    @Override
    public void deleteAsset(String name) throws IOException {
        backend.call(header("AssetDelete", name));
    }

    private JsonObject header(String type, String name) {
        JsonObject header = Msg.of(type);
        header.addProperty("map", mapId);
        header.addProperty("name", Objects.requireNonNull(name, "name"));
        return header;
    }

    /** Upstream {@code MapStorage.escapeAssetName}. */
    static String escapeAssetName(String name) {
        return name
                .replaceAll("[^\\w\\d.\\-_/]", "_")
                .replace("..", "_.");
    }

}
