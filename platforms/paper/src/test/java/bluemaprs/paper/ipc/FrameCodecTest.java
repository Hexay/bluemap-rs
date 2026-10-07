package bluemaprs.paper.ipc;

import com.google.gson.JsonObject;
import org.junit.jupiter.api.Test;

import java.io.ByteArrayInputStream;
import java.io.ByteArrayOutputStream;
import java.io.EOFException;
import java.io.IOException;
import java.util.Arrays;

import static java.nio.charset.StandardCharsets.UTF_8;
import static org.junit.jupiter.api.Assertions.assertArrayEquals;
import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertThrows;

class FrameCodecTest {

    @Test
    void exactBytesMatchTheSpec() throws IOException {
        byte[] bytes = FrameCodec.encode(new Frame(Msg.of("X"), "ab".getBytes(UTF_8)));
        byte[] header = "{\"t\":\"X\"}".getBytes(UTF_8);
        byte[] expected = new byte[8 + header.length + 2];
        expected[3] = (byte) (4 + header.length + 2);
        expected[7] = (byte) header.length;
        System.arraycopy(header, 0, expected, 8, header.length);
        expected[expected.length - 2] = 'a';
        expected[expected.length - 1] = 'b';
        assertArrayEquals(expected, bytes);
    }

    @Test
    void roundTripWithAndWithoutBody() throws IOException {
        JsonObject markers = Msg.of("Markers");
        markers.addProperty("map", "w");
        markers.addProperty("more", false);
        ByteArrayOutputStream out = new ByteArrayOutputStream();
        FrameCodec.write(out, new Frame(Msg.of("Shutdown")));
        FrameCodec.write(out, new Frame(markers, "{\"a\":\"<ü>\"}".getBytes(UTF_8)));

        ByteArrayInputStream in = new ByteArrayInputStream(out.toByteArray());
        Frame a = FrameCodec.read(in);
        assertEquals("Shutdown", a.type());
        assertEquals(0, a.body().length);
        Frame b = FrameCodec.read(in);
        assertEquals(markers, b.header());
        assertEquals("{\"a\":\"<ü>\"}", new String(b.body(), UTF_8));
        assertNull(FrameCodec.read(in));
    }

    @Test
    void headersAreNotHtmlEscaped() throws IOException {
        JsonObject header = Msg.of("Command");
        header.addProperty("input", "bluemap <x> = 'y' & z");
        byte[] bytes = FrameCodec.encode(new Frame(header));
        String json = new String(bytes, 8, bytes.length - 8, UTF_8);
        assertEquals("{\"t\":\"Command\",\"input\":\"bluemap <x> = 'y' & z\"}", json);
    }

    @Test
    void truncationAnywhereIsAnError() throws IOException {
        byte[] full = FrameCodec.encode(new Frame(Msg.of("X"), "body".getBytes(UTF_8)));
        for (int cut = 1; cut < full.length; cut++) {
            byte[] part = Arrays.copyOf(full, cut);
            assertThrows(EOFException.class, () -> FrameCodec.read(new ByteArrayInputStream(part)), "cut at " + cut);
        }
    }

    @Test
    void rejectsBadLengthsAndHeaders() {
        byte[] shortFrame = {0, 0, 0, 3, 0, 0, 0, 0};
        assertThrows(IOException.class, () -> FrameCodec.read(new ByteArrayInputStream(shortFrame)));
        byte[] huge = {-1, -1, -1, -1, 0, 0, 0, 2};
        assertThrows(IOException.class, () -> FrameCodec.read(new ByteArrayInputStream(huge)));
        byte[] noType = {0, 0, 0, 12, 0, 0, 0, 8, '{', '"', 'i', 'd', '"', ':', '1', '}'};
        assertThrows(IOException.class, () -> FrameCodec.read(new ByteArrayInputStream(noType)));
    }

}
