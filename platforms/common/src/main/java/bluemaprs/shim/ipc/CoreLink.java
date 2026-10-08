package bluemaprs.shim.ipc;

import com.google.gson.JsonObject;

import java.io.IOException;
import java.util.List;
import java.util.concurrent.CompletableFuture;
import java.util.concurrent.atomic.AtomicLong;

/** The connection to the current core, or none while it is down; survives core restarts. */
public final class CoreLink {

    private final AtomicLong ids = new AtomicLong();
    private volatile CoreConnection connection;

    /** Called once the handshake ({@code Welcome}) completed. */
    public void attach(CoreConnection connection) {
        this.connection = connection;
    }

    public synchronized void detach(CoreConnection connection) {
        if (this.connection == connection) this.connection = null;
    }

    public boolean connected() {
        return connection != null;
    }

    public long nextId() {
        return ids.incrementAndGet();
    }

    public void send(Frame frame) throws IOException {
        CoreConnection c = connection;
        if (c == null) throw new IOException(CoreConnection.DOWN);
        c.send(frame);
    }

    /** Dropped while the core is down; callers replay after a reconnect. */
    public void sendLatest(String key, List<Frame> frames) {
        CoreConnection c = connection;
        if (c != null) c.sendLatest(key, frames);
    }

    /** Assigns the id; completes exceptionally with an {@link IOException} while the core is down. */
    public CompletableFuture<Reply> request(JsonObject header, byte[] body) {
        CoreConnection c = connection;
        if (c == null) return CompletableFuture.failedFuture(new IOException(CoreConnection.DOWN));
        return c.request(nextId(), header, body);
    }

}
