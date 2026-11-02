import java.io.BufferedReader;
import java.io.InputStreamReader;
import java.net.URL;
import javax.net.ssl.HttpsURLConnection;

/** One bounded system-trust HTTPS observation; no custom trust/hostname verifier. */
public final class Mio82TlsTrace {
    public static void main(String[] args) throws Exception {
        long started = System.nanoTime();
        HttpsURLConnection connection = null;
        int exit = 1;
        try {
            connection = (HttpsURLConnection) new URL(
                "https://www.cloudflare.com/cdn-cgi/trace").openConnection();
            connection.setConnectTimeout(3000);
            connection.setReadTimeout(6000);
            connection.setInstanceFollowRedirects(false);
            int status = connection.getResponseCode();
            System.out.println("httpStatus=" + status);
            if (status != 200) throw new IllegalStateException("unexpected HTTP status");
            System.out.println("normalTlsVerification=true");
            System.out.println("cipherSuite=" + connection.getCipherSuite());
            try (BufferedReader input = new BufferedReader(new InputStreamReader(
                    connection.getInputStream(), "UTF-8"))) {
                String line;
                int lines = 0;
                while ((line = input.readLine()) != null && ++lines <= 64) {
                    if (line.startsWith("ip=") || line.startsWith("warp=")
                            || line.startsWith("tls=") || line.startsWith("http=")) {
                        System.out.println(line);
                    }
                }
            }
            exit = 0;
        } catch (Exception error) {
            System.out.println("failureClass=" + error.getClass().getSimpleName());
        } finally {
            System.out.println("callerElapsedMs=" + (System.nanoTime() - started) / 1000000.0);
            long cleanup = System.nanoTime();
            if (connection != null) connection.disconnect();
            System.out.println("cleanupElapsedMs=" + (System.nanoTime() - cleanup) / 1000000.0);
            System.out.println("httpsDisconnected=true");
        }
        System.exit(exit);
    }
}
