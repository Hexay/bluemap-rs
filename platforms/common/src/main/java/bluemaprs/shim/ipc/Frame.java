package bluemaprs.shim.ipc;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

/** One IPC frame: a JSON header whose {@code t} names the message, plus a raw body (see bm-ipc's crate docs). */
public record Frame(JsonObject header, byte[] body) {

    public static final byte[] EMPTY = new byte[0];

    public Frame(JsonObject header) {
        this(header, EMPTY);
    }

    public String type() {
        JsonElement t = header.get("t");
        return t != null && t.isJsonPrimitive() ? t.getAsString() : "";
    }

    public long id() {
        return header.get("id").getAsLong();
    }

}
