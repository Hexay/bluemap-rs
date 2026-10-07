package bluemaprs.shim.ipc;

import com.google.gson.JsonElement;
import com.google.gson.JsonNull;

/** {@code Reply{id, ok, err?, value?}} plus the frame body (e.g. {@code AssetRead} bytes). */
public record Reply(long id, boolean ok, String err, JsonElement value, byte[] body) {

    public static Reply of(Frame frame) {
        JsonElement ok = frame.header().get("ok");
        JsonElement value = frame.header().get("value");
        return new Reply(
                frame.id(),
                ok != null && ok.getAsBoolean(),
                Msg.string(frame.header(), "err"),
                value == null ? JsonNull.INSTANCE : value,
                frame.body()
        );
    }

    public boolean bool(boolean fallback) {
        return value.isJsonPrimitive() ? value.getAsBoolean() : fallback;
    }

}
