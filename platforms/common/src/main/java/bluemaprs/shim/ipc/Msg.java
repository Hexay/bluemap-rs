package bluemaprs.shim.ipc;

import com.google.gson.Gson;
import com.google.gson.GsonBuilder;
import com.google.gson.JsonElement;
import com.google.gson.JsonObject;

import java.util.Map;

/** Header building and parsing; field names are the camelCase record components of {@link Proto}. */
public final class Msg {

    /** Must equal bm-ipc's {@code PROTOCOL}. */
    public static final int PROTOCOL = 1;

    public static final Gson GSON = new GsonBuilder().disableHtmlEscaping().create();

    private Msg() {}

    public static JsonObject of(String type) {
        JsonObject header = new JsonObject();
        header.addProperty("t", type);
        return header;
    }

    /** {@code t} plus every non-null field of {@code fields} (a record or map). */
    public static JsonObject of(String type, Object fields) {
        JsonObject header = of(type);
        for (Map.Entry<String, JsonElement> e : GSON.toJsonTree(fields).getAsJsonObject().entrySet())
            header.add(e.getKey(), e.getValue());
        return header;
    }

    public static <T> T parse(JsonObject header, Class<T> type) {
        return GSON.fromJson(header, type);
    }

    public static String string(JsonObject header, String field) {
        JsonElement e = header.get(field);
        return e == null || e.isJsonNull() ? null : e.getAsString();
    }

    public static JsonObject reply(long id, boolean ok, String err, JsonElement value) {
        JsonObject header = of("Reply");
        header.addProperty("id", id);
        header.addProperty("ok", ok);
        if (err != null) header.addProperty("err", err);
        if (value != null) header.add("value", value);
        return header;
    }

}
