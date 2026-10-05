// SPDX-License-Identifier: MIT
#include <gst/play/play.h>
#include <stdio.h>

static GMutex lock;
static GCond cond;
static gboolean reached, release_message, repeat_dispose;
static gint finalizations;
static GThread *worker_thread;

static void
finalized(gpointer data, GObject *object)
{
  (void)data;
  (void)object;
  g_atomic_int_inc(&finalizations);
}

static GstBusSyncReply
on_message(GstBus *bus, GstMessage *message, gpointer data)
{
  (void)bus;
  (void)data;
  g_mutex_lock(&lock);
  if (!reached) {
    worker_thread = g_thread_ref(g_thread_self());
    if (repeat_dispose) {
      g_object_run_dispose(G_OBJECT(GST_MESSAGE_SRC(message)));
      g_object_run_dispose(G_OBJECT(GST_MESSAGE_SRC(message)));
    }
    reached = TRUE;
    g_cond_signal(&cond);
    while (!release_message)
      g_cond_wait(&cond, &lock);
  }
  g_mutex_unlock(&lock);
  gst_message_unref(message);
  return GST_BUS_DROP;
}

int
main(int argc, char **argv)
{
  gst_init(&argc, &argv);
  if (argc != 3 || (!g_str_equal(argv[2], "worker") &&
                    !g_str_equal(argv[2], "repeat-dispose") &&
                    !g_str_equal(argv[2], "main-dispose"))) {
    fprintf(stderr, "Usage: %s /absolute/video.mp4 worker|repeat-dispose|main-dispose\n", argv[0]);
    return 2;
  }
  repeat_dispose = argc > 2 && g_str_equal(argv[2], "repeat-dispose");
  GstPlay *play = gst_play_new(NULL);
  g_object_weak_ref(G_OBJECT(play), finalized, NULL);
  GstElement *pipeline = NULL;
  g_object_get(play, "pipeline", &pipeline, NULL);
  GstElement *video = gst_element_factory_make("fakesink", NULL);
  GstElement *audio = gst_element_factory_make("fakesink", NULL);
  if (!video || !audio)
    return 2;
  g_object_set(pipeline, "video-sink", video, "audio-sink", audio, NULL);
  gst_object_unref(pipeline);
  GstBus *bus = gst_play_get_message_bus(play);
  gst_bus_set_flushing(bus, TRUE);
  if (argc > 2 && g_str_equal(argv[2], "main-dispose")) {
    gst_object_unref(play);
    gst_object_unref(bus);
    g_assert_cmpint(g_atomic_int_get(&finalizations), ==, 1);
    puts("PASS: main-thread disposal joined and finalized once");
    return 0;
  }
  gst_bus_set_flushing(bus, FALSE);
  gst_bus_set_sync_handler(bus, on_message, NULL, NULL);
  gchar *uri = g_filename_to_uri(argv[1], NULL, NULL);
  if (!uri)
    return 2;
  gst_play_set_uri(play, uri);
  g_free(uri);
  gst_play_play(play);
  g_mutex_lock(&lock);
  gint64 deadline = g_get_monotonic_time() + 5 * G_TIME_SPAN_SECOND;
  while (!reached) {
    if (!g_cond_wait_until(&cond, &lock, deadline)) {
      g_mutex_unlock(&lock);
      return 2;
    }
  }
  g_mutex_unlock(&lock);

  /* The already-in-flight message survives flushing, but queued messages do not. */
  gst_bus_set_flushing(bus, TRUE);
  gst_object_unref(play);
  g_mutex_lock(&lock);
  release_message = TRUE;
  g_cond_signal(&cond);
  g_mutex_unlock(&lock);

  /* Own-thread disposal does not join itself. This reference lets the test
   * join it and verify completion, not merely an early weak notification. */
  g_thread_join(worker_thread);
  gst_object_unref(bus);
  g_assert_cmpint(g_atomic_int_get(&finalizations), ==, 1);
  g_mutex_clear(&lock);
  g_cond_clear(&cond);
  puts("PASS: worker exited and GstPlay finalized once");
  return 0;
}
