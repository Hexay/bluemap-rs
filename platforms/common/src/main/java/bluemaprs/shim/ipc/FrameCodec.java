package bluemaprs.shim.ipc;

import com.google.gson.JsonElement;
import com.google.gson.JsonObject;
import com.google.gson.JsonParseException;
import com.google.gson.JsonParser;

import java.io.EOFException;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.nio.ByteBuffer;

import static java.nio.charset.StandardCharsets.UTF_8;

/** {@code u32 BE frame_len = 4 + header_len + body_len | u32 BE header_len | header JSON | body}. */
public final class FrameCodec {

    public static final int MAX_HEADER = 1 << 20;
    public static final int MAX_BODY = 64 << 20;

    private FrameCodec() {}

    public static byte[] encode(Frame frame) throws IOException {
        byte[] header = Msg.GSON.toJson(frame.header()).getBytes(UTF_8);
        byte[] body = frame.body();
        if (header.length > MAX_HEADER || body.length > MAX_BODY)
            throw new IOException("frame too large: header " + header.length + " B, body " + body.length + " B");
        return ByteBuffer.allocate(8 + header.length + body.length)
                .putInt(4 + header.length + body.length)
                .putInt(header.length)
                .put(header)
                .put(body)
                .array();
    }

    public static void write(OutputStream out, Frame frame) throws IOException {
        out.write(encode(frame));
    }

    /** The next frame, or {@code null} on a clean EOF between frames. */
    public static Frame read(InputStream in) throws IOException {
        byte[] len = new byte[4];
        int got = 0;
        while (got < 4) {
            int n = in.read(len, got, 4 - got);
            if (n < 0) {
                if (got == 0) return null;
                throw new EOFException("stream ended inside a frame");
            }
            got += n;
        }
        long frameLen = Integer.toUnsignedLong(ByteBuffer.wrap(len).getInt());
        if (frameLen < 4) throw new IOException("malformed frame length " + frameLen);
        long headerLen = Integer.toUnsignedLong(ByteBuffer.wrap(readExactly(in, 4)).getInt());
        long rest = frameLen - 4;
        if (headerLen > rest) throw new IOException("malformed frame length " + frameLen);
        long bodyLen = rest - headerLen;
        if (headerLen > MAX_HEADER || bodyLen > MAX_BODY)
            throw new IOException("frame too large: header " + headerLen + " B, body " + bodyLen + " B");

        byte[] header = readExactly(in, (int) headerLen);
        byte[] body = bodyLen == 0 ? Frame.EMPTY : readExactly(in, (int) bodyLen);
        return new Frame(parseHeader(header), body);
    }

    private static JsonObject parseHeader(byte[] bytes) throws IOException {
        JsonElement json;
        try {
            json = JsonParser.parseString(new String(bytes, UTF_8));
        } catch (JsonParseException e) {
            throw new IOException("malformed header: " + e.getMessage(), e);
        }
        if (json instanceof JsonObject obj
                && obj.get("t") instanceof JsonElement t && t.isJsonPrimitive() && t.getAsJsonPrimitive().isString())
            return obj;
        throw new IOException("header has no string field \"t\"");
    }

    private static byte[] readExactly(InputStream in, int len) throws IOException {
        byte[] buf = in.readNBytes(len);
        if (buf.length < len) throw new EOFException("stream ended inside a frame");
        return buf;
    }

}
