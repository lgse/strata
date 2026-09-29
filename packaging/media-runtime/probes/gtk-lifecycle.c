// SPDX-License-Identifier: MIT
#include <gtk/gtk.h>
#include <dirent.h>
#include <stdio.h>

static GtkMediaFile *media;
static GtkWidget *picture;
static GMainLoop *loop;
static GBytes *bytes;
static guint cycles, finalizations, limit = 50;
static gboolean playing, failed;

static void
finalized(gpointer data, GObject *object)
{
  (void)data;
  (void)object;
  finalizations++;
}

static int
count_entries(const char *path)
{
  DIR *directory = opendir(path);
  if (!directory)
    return -1;
  int count = 0;
  struct dirent *entry;
  while ((entry = readdir(directory)))
    if (entry->d_name[0] != '.')
      count++;
  closedir(directory);
  return count;
}

static gboolean
tick(gpointer data)
{
  (void)data;
  if (!playing) {
    int fds = count_entries("/proc/self/fd");
    int threads = count_entries("/proc/self/task");
    printf("checkpoint %u fds=%d threads=%d finalized=%u\n",
           cycles, fds, threads, finalizations);
    fflush(stdout);
    if (fds < 0 || threads < 0 || finalizations != cycles) {
      failed = TRUE;
      g_main_loop_quit(loop);
      return G_SOURCE_REMOVE;
    }
    if (cycles == limit) {
      g_main_loop_quit(loop);
      return G_SOURCE_REMOVE;
    }
    GInputStream *input = g_memory_input_stream_new_from_bytes(bytes);
    media = GTK_MEDIA_FILE(gtk_media_file_new_for_input_stream(input));
    g_object_unref(input);
    g_object_weak_ref(G_OBJECT(media), finalized, NULL);
    gtk_picture_set_paintable(GTK_PICTURE(picture), GDK_PAINTABLE(media));
    gtk_media_stream_set_loop(GTK_MEDIA_STREAM(media), TRUE);
    gtk_media_stream_play(GTK_MEDIA_STREAM(media));
    playing = TRUE;
  } else {
    if (!gtk_media_stream_is_prepared(GTK_MEDIA_STREAM(media)) ||
        gtk_media_stream_get_error(GTK_MEDIA_STREAM(media))) {
      fprintf(stderr, "Media failed to prepare; this is not a passing lifecycle test\n");
      failed = TRUE;
    }
    gtk_media_stream_pause(GTK_MEDIA_STREAM(media));
    gtk_media_file_clear(media);
    gtk_picture_set_paintable(GTK_PICTURE(picture), NULL);
    g_clear_object(&media);
    playing = FALSE;
    cycles++;
    if (failed) {
      g_main_loop_quit(loop);
      return G_SOURCE_REMOVE;
    }
  }
  return G_SOURCE_CONTINUE;
}

int
main(int argc, char **argv)
{
  if (argc != 3) {
    fprintf(stderr, "Usage: %s video.mp4 cycles\n", argv[0]);
    return 2;
  }
  guint64 requested;
  if (!g_ascii_string_to_unsigned(argv[2], 10, 1, 1000, &requested, NULL))
    return 2;
  limit = (guint)requested;
  gtk_init();
  char *contents;
  gsize length;
  if (!g_file_get_contents(argv[1], &contents, &length, NULL))
    return 2;
  bytes = g_bytes_new_take(contents, length);
  GtkWidget *window = gtk_window_new();
  picture = gtk_picture_new();
  gtk_window_set_child(GTK_WINDOW(window), picture);
  gtk_window_present(GTK_WINDOW(window));
  loop = g_main_loop_new(NULL, FALSE);
  g_timeout_add(1000, tick, NULL);
  g_main_loop_run(loop);
  gtk_window_destroy(GTK_WINDOW(window));
  g_bytes_unref(bytes);
  g_main_loop_unref(loop);
  return failed ? 1 : 0;
}
