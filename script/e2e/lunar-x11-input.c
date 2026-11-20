/*
 * lunar-x11-input : pilote XTest des harnais E2E X11 de LUNAR.
 *
 * Licence du projet : AGPL-3.0-only (voir LICENSE.md).
 *
 * Commandes propres aux fenêtres gpui et aux dialogues rfd :
 *   - wait-dialog reconnaît aussi un dialogue par le _NET_WM_PID de la
 *     fenêtre principale (rfd ouvre le sélecteur GTK sans parent ni classe
 *     commune avec la fenêtre gpui) ;
 *   - type-id : saisie d'un texte ASCII (Maj ajoutée quand la touche
 *     l'exige) ;
 *   - paste-id : collage par le presse-papiers, servi par le pilote
 *     lui-même (remplace xclip, qui restait en arrière-plan) ;
 *   - shift-id : Maj+touche (Maj+Tab) ;
 *   - double-click-id, wheel-id (molette), move-id (survol) ;
 *   - drop-id : dépôt d'un fichier par glisser-déposer (XDND), servi par le
 *     pilote comme source ;
 *   - list-windows : fenêtres visibles (diagnostic) ;
 *   - pid-id : _NET_WM_PID d'une fenêtre.
 */

#include <X11/Xatom.h>
#include <X11/Xlib.h>
#include <X11/Xutil.h>
#include <X11/XKBlib.h>
#include <X11/extensions/XTest.h>

#include <errno.h>
#include <limits.h>
#include <stdbool.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <time.h>
#include <unistd.h>

typedef struct {
  Window window;
  unsigned long area;
} WindowMatch;

static volatile int x_error_occurred = 0;

static int ignore_x_error(Display *display, XErrorEvent *error) {
  (void)display;
  (void)error;
  x_error_occurred = 1;
  return 0;
}

static void die(const char *message) {
  fprintf(stderr, "lunar-x11-input: %s\n", message);
  exit(2);
}

static long parse_long(const char *value, const char *label) {
  char *end = NULL;
  errno = 0;
  long parsed = strtol(value, &end, 0);
  if (errno != 0 || end == value || *end != '\0') {
    fprintf(stderr, "lunar-x11-input: invalid %s: %s\n", label, value);
    exit(2);
  }
  return parsed;
}

static Window parse_window(const char *value) {
  long parsed = parse_long(value, "window id");
  if (parsed <= 0) {
    die("window id must be positive");
  }
  return (Window)parsed;
}

static long monotonic_millis(void) {
  struct timespec value;
  if (clock_gettime(CLOCK_MONOTONIC, &value) != 0) {
    die("clock_gettime failed");
  }
  return value.tv_sec * 1000L + value.tv_nsec / 1000000L;
}

static char *window_name(Display *display, Window window) {
  Atom utf8 = XInternAtom(display, "UTF8_STRING", False);
  Atom net_wm_name = XInternAtom(display, "_NET_WM_NAME", False);
  Atom actual_type;
  int actual_format;
  unsigned long item_count;
  unsigned long bytes_after;
  unsigned char *property = NULL;

  if (XGetWindowProperty(display, window, net_wm_name, 0, 4096, False, utf8,
                         &actual_type, &actual_format, &item_count,
                         &bytes_after, &property) == Success &&
      property != NULL && actual_format == 8) {
    char *result = strdup((const char *)property);
    XFree(property);
    return result;
  }
  if (property != NULL) {
    XFree(property);
  }

  char *legacy_name = NULL;
  if (XFetchName(display, window, &legacy_name) != 0 && legacy_name != NULL) {
    char *result = strdup(legacy_name);
    XFree(legacy_name);
    return result;
  }
  return NULL;
}

static bool is_viewable_large_window(Display *display, Window window,
                                     XWindowAttributes *attributes) {
  if (XGetWindowAttributes(display, window, attributes) == 0) {
    return false;
  }
  return attributes->map_state == IsViewable && attributes->width >= 100 &&
         attributes->height >= 100;
}

static void find_named_recursive(Display *display, Window parent,
                                 const char *needle, WindowMatch *best) {
  Window root;
  Window parent_return;
  Window *children = NULL;
  unsigned int child_count = 0;

  if (XQueryTree(display, parent, &root, &parent_return, &children,
                 &child_count) == 0) {
    return;
  }

  for (unsigned int index = 0; index < child_count; index++) {
    Window child = children[index];
    XWindowAttributes attributes;
    if (is_viewable_large_window(display, child, &attributes)) {
      char *name = window_name(display, child);
      if (name != NULL && strstr(name, needle) != NULL) {
        unsigned long area =
            (unsigned long)attributes.width * (unsigned long)attributes.height;
        if (area > best->area) {
          best->window = child;
          best->area = area;
        }
      }
      free(name);
    }
    find_named_recursive(display, child, needle, best);
  }

  if (children != NULL) {
    XFree(children);
  }
}

static Window find_named_window(Display *display, const char *needle) {
  WindowMatch best = {0, 0};
  find_named_recursive(display, DefaultRootWindow(display), needle, &best);
  return best.window;
}

static bool same_window_class(Display *display, Window left, Window right) {
  XClassHint left_hint = {0};
  XClassHint right_hint = {0};
  bool same = false;

  if (XGetClassHint(display, left, &left_hint) != 0 &&
      XGetClassHint(display, right, &right_hint) != 0) {
    const char *left_name = left_hint.res_name == NULL ? "" : left_hint.res_name;
    const char *right_name =
        right_hint.res_name == NULL ? "" : right_hint.res_name;
    const char *left_class =
        left_hint.res_class == NULL ? "" : left_hint.res_class;
    const char *right_class =
        right_hint.res_class == NULL ? "" : right_hint.res_class;
    same = strcmp(left_name, right_name) == 0 &&
           strcmp(left_class, right_class) == 0;
  }

  if (left_hint.res_name != NULL) {
    XFree(left_hint.res_name);
  }
  if (left_hint.res_class != NULL) {
    XFree(left_hint.res_class);
  }
  if (right_hint.res_name != NULL) {
    XFree(right_hint.res_name);
  }
  if (right_hint.res_class != NULL) {
    XFree(right_hint.res_class);
  }
  return same;
}

static long window_pid(Display *display, Window window) {
  Atom net_wm_pid = XInternAtom(display, "_NET_WM_PID", False);
  Atom actual_type;
  int actual_format;
  unsigned long item_count;
  unsigned long bytes_after;
  unsigned char *property = NULL;
  long pid = 0;
  if (XGetWindowProperty(display, window, net_wm_pid, 0, 1, False,
                         XA_CARDINAL, &actual_type, &actual_format,
                         &item_count, &bytes_after, &property) == Success &&
      property != NULL && actual_format == 32 && item_count == 1) {
    pid = (long)*(unsigned long *)property;
  }
  if (property != NULL) {
    XFree(property);
  }
  return pid;
}

static void find_dialog_recursive(Display *display, Window parent, Window main,
                                  long main_pid, WindowMatch *best) {
  Window root;
  Window parent_return;
  Window *children = NULL;
  unsigned int child_count = 0;

  if (XQueryTree(display, parent, &root, &parent_return, &children,
                 &child_count) == 0) {
    return;
  }

  for (unsigned int index = 0; index < child_count; index++) {
    Window child = children[index];
    XWindowAttributes attributes;
    if (child != main && is_viewable_large_window(display, child, &attributes)) {
      Window transient_for = None;
      bool is_transient =
          XGetTransientForHint(display, child, &transient_for) != 0 &&
          transient_for == main;
      bool same_pid =
          main_pid > 0 && window_pid(display, child) == main_pid;
      if (is_transient || same_pid || same_window_class(display, child, main)) {
        char *name = window_name(display, child);
        if (name != NULL) {
          unsigned long area = (unsigned long)attributes.width *
                               (unsigned long)attributes.height;
          if (area > best->area) {
            best->window = child;
            best->area = area;
          }
        }
        free(name);
      }
    }
    find_dialog_recursive(display, child, main, main_pid, best);
  }

  if (children != NULL) {
    XFree(children);
  }
}

static Window find_dialog(Display *display, Window main) {
  WindowMatch best = {0, 0};
  find_dialog_recursive(display, DefaultRootWindow(display), main,
                        window_pid(display, main), &best);
  return best.window;
}

static Window wait_for_named_window(Display *display, const char *needle,
                                    long timeout_ms) {
  long deadline = monotonic_millis() + timeout_ms;
  do {
    Window found = find_named_window(display, needle);
    if (found != None) {
      return found;
    }
    usleep(100000);
  } while (monotonic_millis() < deadline);
  return None;
}

static Window wait_for_dialog(Display *display, Window main, long timeout_ms) {
  long deadline = monotonic_millis() + timeout_ms;
  do {
    Window found = find_dialog(display, main);
    if (found != None) {
      return found;
    }
    usleep(100000);
  } while (monotonic_millis() < deadline);
  return None;
}

static bool window_exists(Display *display, Window window) {
  XWindowAttributes attributes;
  x_error_occurred = 0;
  int success = XGetWindowAttributes(display, window, &attributes);
  XSync(display, False);
  return success != 0 && x_error_occurred == 0 &&
         attributes.map_state == IsViewable;
}

static bool window_manager_is_running(Display *display) {
  Atom wm_check = XInternAtom(display, "_NET_SUPPORTING_WM_CHECK", False);
  Atom actual_type;
  int actual_format;
  unsigned long item_count;
  unsigned long bytes_after;
  unsigned char *property = NULL;
  int result = XGetWindowProperty(
      display, DefaultRootWindow(display), wm_check, 0, 1, False, XA_WINDOW,
      &actual_type, &actual_format, &item_count, &bytes_after, &property);
  bool running = result == Success && property != NULL && item_count == 1;
  if (property != NULL) {
    XFree(property);
  }
  return running;
}

static void focus_window(Display *display, Window window) {
  Window root = DefaultRootWindow(display);
  if (window_manager_is_running(display)) {
    Atom active_window = XInternAtom(display, "_NET_ACTIVE_WINDOW", False);
    XEvent event;
    memset(&event, 0, sizeof(event));
    event.xclient.type = ClientMessage;
    event.xclient.window = window;
    event.xclient.message_type = active_window;
    event.xclient.format = 32;
    event.xclient.data.l[0] = 2;
    event.xclient.data.l[1] = CurrentTime;
    XSendEvent(display, root, False,
               SubstructureRedirectMask | SubstructureNotifyMask, &event);
  } else {
    XRaiseWindow(display, window);
    XSetInputFocus(display, window, RevertToParent, CurrentTime);
  }
  XSync(display, False);
  usleep(200000);
}

static void fake_key(Display *display, KeySym symbol, Bool pressed) {
  KeyCode code = XKeysymToKeycode(display, symbol);
  if (code == 0) {
    fprintf(stderr, "lunar-x11-input: missing keycode for keysym %lu\n",
            symbol);
    exit(2);
  }
  XTestFakeKeyEvent(display, code, pressed, CurrentTime);
}

static void send_key(Display *display, Window window, const char *key_name,
                     bool control, bool shift) {
  KeySym symbol = XStringToKeysym(key_name);
  if (symbol == NoSymbol) {
    fprintf(stderr, "lunar-x11-input: unknown keysym: %s\n", key_name);
    exit(2);
  }

  // Le script appelle focus-id lors d'un changement de fenêtre. Ne pas
  // réassigner ici le focus X au top-level : cela ferait perdre au dialogue
  // GTK le focus interne posé par Ctrl+L sur son champ de chemin.
  (void)window;
  if (control) {
    fake_key(display, XK_Control_L, True);
  }
  if (shift) {
    fake_key(display, XK_Shift_L, True);
  }
  fake_key(display, symbol, True);
  fake_key(display, symbol, False);
  if (shift) {
    fake_key(display, XK_Shift_L, False);
  }
  if (control) {
    fake_key(display, XK_Control_L, False);
  }
  XSync(display, False);
  usleep(100000);
}

static void click_window(Display *display, Window window, int x, int y) {
  XWindowAttributes attributes;
  if (XGetWindowAttributes(display, window, &attributes) == 0) {
    die("cannot read target window geometry");
  }
  if (x < 0 || y < 0 || x >= attributes.width || y >= attributes.height) {
    die("click coordinates are outside the target window");
  }

  Window child;
  int root_x;
  int root_y;
  if (XTranslateCoordinates(display, window, DefaultRootWindow(display), x, y,
                            &root_x, &root_y, &child) == 0) {
    die("cannot translate click coordinates");
  }

  focus_window(display, window);
  XTestFakeMotionEvent(display, DefaultScreen(display), root_x, root_y,
                       CurrentTime);
  XTestFakeButtonEvent(display, 1, True, CurrentTime);
  XTestFakeButtonEvent(display, 1, False, CurrentTime);
  XSync(display, False);
  usleep(100000);
}

/* Point (x, y) de la fenêtre en coordonnées de l'écran ; refuse un point
 * hors de la fenêtre. */
static void root_point(Display *display, Window window, int x, int y,
                       int *root_x, int *root_y) {
  XWindowAttributes attributes;
  Window child;
  if (XGetWindowAttributes(display, window, &attributes) == 0) {
    die("cannot read target window geometry");
  }
  if (x < 0 || y < 0 || x >= attributes.width || y >= attributes.height) {
    die("coordinates are outside the target window");
  }
  if (XTranslateCoordinates(display, window, DefaultRootWindow(display), x, y,
                            root_x, root_y, &child) == 0) {
    die("cannot translate coordinates");
  }
}

/* Survol : déplace le pointeur sans cliquer. */
static void move_window(Display *display, Window window, int x, int y) {
  int root_x;
  int root_y;
  root_point(display, window, x, y, &root_x, &root_y);
  XTestFakeMotionEvent(display, DefaultScreen(display), root_x, root_y,
                       CurrentTime);
  XSync(display, False);
  usleep(100000);
}

/* Double clic gauche : deux clics à 60 ms d'écart (sous le délai de double
 * clic de gpui). */
static void double_click_window(Display *display, Window window, int x,
                                int y) {
  int root_x;
  int root_y;
  root_point(display, window, x, y, &root_x, &root_y);
  focus_window(display, window);
  XTestFakeMotionEvent(display, DefaultScreen(display), root_x, root_y,
                       CurrentTime);
  for (int index = 0; index < 2; index++) {
    XTestFakeButtonEvent(display, 1, True, CurrentTime);
    XTestFakeButtonEvent(display, 1, False, CurrentTime);
    XSync(display, False);
    usleep(60000);
  }
  usleep(100000);
}

/* Molette : COUNT crans vers le haut (bouton 4) ou le bas (bouton 5) au
 * point (x, y). */
static void wheel_window(Display *display, Window window, int x, int y,
                         const char *direction, long count) {
  unsigned int button;
  if (strcmp(direction, "up") == 0) {
    button = 4;
  } else if (strcmp(direction, "down") == 0) {
    button = 5;
  } else {
    die("wheel direction must be up or down");
  }
  int root_x;
  int root_y;
  root_point(display, window, x, y, &root_x, &root_y);
  XTestFakeMotionEvent(display, DefaultScreen(display), root_x, root_y,
                       CurrentTime);
  for (long index = 0; index < count; index++) {
    XTestFakeButtonEvent(display, button, True, CurrentTime);
    XTestFakeButtonEvent(display, button, False, CurrentTime);
    XSync(display, False);
    usleep(30000);
  }
  usleep(100000);
}

/* Saisie d'un texte ASCII imprimable : une touche par caractère, Maj
 * ajoutée quand le caractère est au deuxième niveau de sa touche. Le focus
 * n'est pas déplacé (voir send_key). */
static void type_text(Display *display, const char *text) {
  for (const unsigned char *cursor = (const unsigned char *)text;
       *cursor != '\0'; cursor++) {
    if (*cursor < 0x20 || *cursor > 0x7e) {
      die("type-id only accepts printable ASCII");
    }
    /* Les keysyms Latin-1 valent le code du caractère. */
    KeySym symbol = (KeySym)*cursor;
    KeyCode code = XKeysymToKeycode(display, symbol);
    if (code == 0) {
      fprintf(stderr, "lunar-x11-input: no key for character %c\n", *cursor);
      exit(2);
    }
    bool shift = XkbKeycodeToKeysym(display, code, 0, 0) != symbol &&
                 XkbKeycodeToKeysym(display, code, 0, 1) == symbol;
    if (shift) {
      fake_key(display, XK_Shift_L, True);
    }
    XTestFakeKeyEvent(display, code, True, CurrentTime);
    XTestFakeKeyEvent(display, code, False, CurrentTime);
    if (shift) {
      fake_key(display, XK_Shift_L, False);
    }
    XSync(display, False);
    usleep(20000);
  }
  usleep(100000);
}

/* Colle TEXT dans la fenêtre focalisée : le pilote prend le presse-papiers
 * (CLIPBOARD), envoie Ctrl+V puis sert lui-même les demandes de contenu
 * (TARGETS, UTF8_STRING, STRING, TEXT) jusqu'à ce que le texte ait été lu, au
 * plus TIMEOUT_MS. Aucun processus ne reste en arrière-plan (contrairement à
 * xclip) et un chemin long est transmis d'un coup, sans la complétion du
 * sélecteur GTK à chaque caractère. */
static void paste_text(Display *display, Window target, const char *text,
                       long timeout_ms) {
  Window owner = XCreateSimpleWindow(display, DefaultRootWindow(display), 0, 0,
                                     1, 1, 0, 0, 0);
  Atom clipboard = XInternAtom(display, "CLIPBOARD", False);
  Atom utf8 = XInternAtom(display, "UTF8_STRING", False);
  Atom targets = XInternAtom(display, "TARGETS", False);
  Atom text_atom = XInternAtom(display, "TEXT", False);
  XSetSelectionOwner(display, clipboard, owner, CurrentTime);
  XSync(display, False);
  if (XGetSelectionOwner(display, clipboard) != owner) {
    die("cannot own the clipboard");
  }
  send_key(display, target, "v", true, false);

  long deadline = monotonic_millis() + timeout_ms;
  long served_at = 0;
  while (monotonic_millis() < deadline) {
    while (XPending(display) > 0) {
      XEvent event;
      XNextEvent(display, &event);
      if (event.type != SelectionRequest) {
        continue;
      }
      XSelectionRequestEvent *request = &event.xselectionrequest;
      Atom property = request->property != None ? request->property
                                                : request->target;
      XEvent reply;
      memset(&reply, 0, sizeof(reply));
      reply.xselection.type = SelectionNotify;
      reply.xselection.display = request->display;
      reply.xselection.requestor = request->requestor;
      reply.xselection.selection = request->selection;
      reply.xselection.target = request->target;
      reply.xselection.time = request->time;
      reply.xselection.property = None;
      if (request->target == targets) {
        Atom offered[] = {targets, utf8, XA_STRING, text_atom};
        XChangeProperty(display, request->requestor, property, XA_ATOM, 32,
                        PropModeReplace, (unsigned char *)offered, 4);
        reply.xselection.property = property;
      } else if (request->target == utf8 || request->target == XA_STRING ||
                 request->target == text_atom) {
        Atom type = request->target == text_atom ? utf8 : request->target;
        XChangeProperty(display, request->requestor, property, type, 8,
                        PropModeReplace, (const unsigned char *)text,
                        (int)strlen(text));
        reply.xselection.property = property;
        served_at = monotonic_millis();
      }
      XSendEvent(display, request->requestor, False, NoEventMask, &reply);
      XFlush(display);
    }
    if (served_at != 0 && monotonic_millis() - served_at > 300) {
      break;
    }
    usleep(10000);
  }
  XDestroyWindow(display, owner);
  XSync(display, False);
  if (served_at == 0) {
    die("pasted text was never requested");
  }
}

/* Message XDND (format 32) de la source `source` vers `target`. */
static void send_xdnd(Display *display, Window target, Atom type, long l0,
                      long l1, long l2, long l3, long l4) {
  XEvent message;
  memset(&message, 0, sizeof(message));
  message.xclient.type = ClientMessage;
  message.xclient.display = display;
  message.xclient.window = target;
  message.xclient.message_type = type;
  message.xclient.format = 32;
  message.xclient.data.l[0] = l0;
  message.xclient.data.l[1] = l1;
  message.xclient.data.l[2] = l2;
  message.xclient.data.l[3] = l3;
  message.xclient.data.l[4] = l4;
  XSendEvent(display, target, False, NoEventMask, &message);
  XFlush(display);
}

/* URI `file://` d'un chemin absolu, octets hors [A-Za-z0-9/._~-] encodés en
 * %XX. */
static char *file_uri(const char *path) {
  if (path[0] != '/') {
    die("dropped path must be absolute");
  }
  size_t length = strlen(path);
  char *uri = malloc(7 + 3 * length + 3);
  if (uri == NULL) {
    die("out of memory");
  }
  char *out = uri + sprintf(uri, "file://");
  for (const unsigned char *in = (const unsigned char *)path; *in != 0; in++) {
    if ((*in >= 'A' && *in <= 'Z') || (*in >= 'a' && *in <= 'z') ||
        (*in >= '0' && *in <= '9') || strchr("/._~-", *in) != NULL) {
      *out++ = (char)*in;
    } else {
      out += sprintf(out, "%%%02X", *in);
    }
  }
  strcpy(out, "\r\n");
  return uri;
}

/* Glisser-déposer (XDND, version 5) du fichier PATH au point (x, y) de
 * `target`, le pilote jouant la source : pointeur amené au point (gpui lit
 * la position du pointeur), XdndEnter (type text/uri-list), XdndPosition,
 * liste d'URI servie à la demande de la cible, XdndDrop après son
 * XdndStatus, puis attente de XdndFinished, le tout avant TIMEOUT_MS. */
static void drop_file(Display *display, Window target, int x, int y,
                      const char *path, long timeout_ms) {
  int root_x;
  int root_y;
  root_point(display, target, x, y, &root_x, &root_y);
  XTestFakeMotionEvent(display, DefaultScreen(display), root_x, root_y,
                       CurrentTime);
  XSync(display, False);
  usleep(100000);

  Window source = XCreateSimpleWindow(display, DefaultRootWindow(display), 0,
                                      0, 1, 1, 0, 0, 0);
  Atom selection = XInternAtom(display, "XdndSelection", False);
  Atom enter = XInternAtom(display, "XdndEnter", False);
  Atom position = XInternAtom(display, "XdndPosition", False);
  Atom status_atom = XInternAtom(display, "XdndStatus", False);
  Atom drop = XInternAtom(display, "XdndDrop", False);
  Atom finished = XInternAtom(display, "XdndFinished", False);
  Atom copy = XInternAtom(display, "XdndActionCopy", False);
  Atom uri_list = XInternAtom(display, "text/uri-list", False);
  char *uri = file_uri(path);
  XSetSelectionOwner(display, selection, source, CurrentTime);
  XSync(display, False);
  if (XGetSelectionOwner(display, selection) != source) {
    die("cannot own the XdndSelection");
  }

  send_xdnd(display, target, enter, (long)source, 5L << 24, (long)uri_list, 0,
            0);
  send_xdnd(display, target, position, (long)source, 0,
            ((long)root_x << 16) | (root_y & 0xffff), CurrentTime,
            (long)copy);

  long deadline = monotonic_millis() + timeout_ms;
  bool served = false;
  bool accepted = false;
  bool dropped = false;
  bool done = false;
  while (!done && monotonic_millis() < deadline) {
    while (XPending(display) > 0) {
      XEvent event;
      XNextEvent(display, &event);
      if (event.type == SelectionRequest &&
          event.xselectionrequest.selection == selection) {
        XSelectionRequestEvent *request = &event.xselectionrequest;
        Atom property = request->property != None ? request->property
                                                  : request->target;
        XEvent reply;
        memset(&reply, 0, sizeof(reply));
        reply.xselection.type = SelectionNotify;
        reply.xselection.display = request->display;
        reply.xselection.requestor = request->requestor;
        reply.xselection.selection = request->selection;
        reply.xselection.target = request->target;
        reply.xselection.time = request->time;
        reply.xselection.property = None;
        if (request->target == uri_list) {
          XChangeProperty(display, request->requestor, property, uri_list, 8,
                          PropModeReplace, (const unsigned char *)uri,
                          (int)strlen(uri));
          reply.xselection.property = property;
          served = true;
        }
        XSendEvent(display, request->requestor, False, NoEventMask, &reply);
        XFlush(display);
      } else if (event.type == ClientMessage &&
                 event.xclient.message_type == status_atom) {
        accepted = (event.xclient.data.l[1] & 1) != 0;
      } else if (event.type == ClientMessage &&
                 event.xclient.message_type == finished) {
        done = true;
      }
    }
    /* Dépôt une fois la liste lue par la cible (les chemins précèdent le
     * dépôt chez gpui) et la position acceptée. */
    if (served && accepted && !dropped) {
      usleep(200000);
      send_xdnd(display, target, drop, (long)source, 0, CurrentTime, 0, 0);
      dropped = true;
    }
    usleep(10000);
  }
  XDestroyWindow(display, source);
  XSync(display, False);
  free(uri);
  if (!served) {
    die("dropped file list was never requested");
  }
  if (!accepted) {
    die("drop target did not accept the position");
  }
  if (!done) {
    die("drop was not finished by the target");
  }
}

/* Fenêtres visibles d'au moins 100×100 : identifiant, taille, PID, titre. */
static void list_windows_recursive(Display *display, Window parent) {
  Window root;
  Window parent_return;
  Window *children = NULL;
  unsigned int child_count = 0;
  if (XQueryTree(display, parent, &root, &parent_return, &children,
                 &child_count) == 0) {
    return;
  }
  for (unsigned int index = 0; index < child_count; index++) {
    Window child = children[index];
    XWindowAttributes attributes;
    if (is_viewable_large_window(display, child, &attributes)) {
      char *name = window_name(display, child);
      printf("0x%lx %dx%d pid=%ld %s\n", child, attributes.width,
             attributes.height, window_pid(display, child),
             name == NULL ? "(sans titre)" : name);
      free(name);
    }
    list_windows_recursive(display, child);
  }
  if (children != NULL) {
    XFree(children);
  }
}

static void print_geometry(Display *display, Window window) {
  XWindowAttributes attributes;
  Window child;
  int root_x;
  int root_y;
  if (XGetWindowAttributes(display, window, &attributes) == 0 ||
      XTranslateCoordinates(display, window, DefaultRootWindow(display), 0, 0,
                            &root_x, &root_y, &child) == 0) {
    die("cannot read target window geometry");
  }
  printf("%d %d %d %d\n", attributes.width, attributes.height, root_x,
         root_y);
}

static void print_usage(const char *program) {
  fprintf(stderr,
          "usage:\n"
          "  %s wait-window TITLE TIMEOUT_MS\n"
          "  %s wait-dialog MAIN_WINDOW_ID TIMEOUT_MS\n"
          "  %s wait-gone WINDOW_ID TIMEOUT_MS\n"
          "  %s geometry-id WINDOW_ID\n"
          "  %s focus-id WINDOW_ID\n"
          "  %s click-id WINDOW_ID X Y\n"
          "  %s key-id WINDOW_ID KEYSYM\n"
          "  %s ctrl-id WINDOW_ID KEYSYM\n"
          "  %s ctrl-shift-id WINDOW_ID KEYSYM\n"
          "  %s shift-id WINDOW_ID KEYSYM\n"
          "  %s type-id WINDOW_ID TEXT\n"
          "  %s paste-id WINDOW_ID TEXT [TIMEOUT_MS]\n"
          "  %s move-id WINDOW_ID X Y\n"
          "  %s double-click-id WINDOW_ID X Y\n"
          "  %s wheel-id WINDOW_ID X Y up|down COUNT\n"
          "  %s drop-id WINDOW_ID X Y ABSOLUTE_PATH [TIMEOUT_MS]\n"
          "  %s pid-id WINDOW_ID\n"
          "  %s list-windows\n",
          program, program, program, program, program, program, program,
          program, program, program, program, program, program, program,
          program, program, program, program);
}

int main(int argc, char **argv) {
  Display *display = XOpenDisplay(NULL);
  if (display == NULL) {
    die("cannot open X display");
  }
  XSetErrorHandler(ignore_x_error);

  int status = 0;
  if (argc == 4 && strcmp(argv[1], "wait-window") == 0) {
    long timeout_ms = parse_long(argv[3], "timeout");
    Window found = wait_for_named_window(display, argv[2], timeout_ms);
    if (found == None) {
      fprintf(stderr, "lunar-x11-input: window not found: %s\n", argv[2]);
      status = 1;
    } else {
      printf("0x%lx\n", found);
    }
  } else if (argc == 4 && strcmp(argv[1], "wait-dialog") == 0) {
    Window main = parse_window(argv[2]);
    long timeout_ms = parse_long(argv[3], "timeout");
    Window found = wait_for_dialog(display, main, timeout_ms);
    if (found == None) {
      fputs("lunar-x11-input: native dialog not found\n", stderr);
      status = 1;
    } else {
      printf("0x%lx\n", found);
    }
  } else if (argc == 4 && strcmp(argv[1], "wait-gone") == 0) {
    Window window = parse_window(argv[2]);
    long timeout_ms = parse_long(argv[3], "timeout");
    long deadline = monotonic_millis() + timeout_ms;
    while (window_exists(display, window) && monotonic_millis() < deadline) {
      usleep(100000);
    }
    if (window_exists(display, window)) {
      fputs("lunar-x11-input: window did not close\n", stderr);
      status = 1;
    }
  } else if (argc == 3 && strcmp(argv[1], "geometry-id") == 0) {
    print_geometry(display, parse_window(argv[2]));
  } else if (argc == 3 && strcmp(argv[1], "focus-id") == 0) {
    focus_window(display, parse_window(argv[2]));
  } else if (argc == 5 && strcmp(argv[1], "click-id") == 0) {
    click_window(display, parse_window(argv[2]),
                 (int)parse_long(argv[3], "x coordinate"),
                 (int)parse_long(argv[4], "y coordinate"));
  } else if (argc == 4 && strcmp(argv[1], "key-id") == 0) {
    send_key(display, parse_window(argv[2]), argv[3], false, false);
  } else if (argc == 4 && strcmp(argv[1], "ctrl-id") == 0) {
    send_key(display, parse_window(argv[2]), argv[3], true, false);
  } else if (argc == 4 && strcmp(argv[1], "ctrl-shift-id") == 0) {
    send_key(display, parse_window(argv[2]), argv[3], true, true);
  } else if (argc == 4 && strcmp(argv[1], "shift-id") == 0) {
    send_key(display, parse_window(argv[2]), argv[3], false, true);
  } else if (argc == 4 && strcmp(argv[1], "type-id") == 0) {
    (void)parse_window(argv[2]);
    type_text(display, argv[3]);
  } else if ((argc == 4 || argc == 5) && strcmp(argv[1], "paste-id") == 0) {
    paste_text(display, parse_window(argv[2]), argv[3],
               argc == 5 ? parse_long(argv[4], "timeout") : 5000);
  } else if (argc == 5 && strcmp(argv[1], "move-id") == 0) {
    move_window(display, parse_window(argv[2]),
                (int)parse_long(argv[3], "x coordinate"),
                (int)parse_long(argv[4], "y coordinate"));
  } else if (argc == 5 && strcmp(argv[1], "double-click-id") == 0) {
    double_click_window(display, parse_window(argv[2]),
                        (int)parse_long(argv[3], "x coordinate"),
                        (int)parse_long(argv[4], "y coordinate"));
  } else if (argc == 7 && strcmp(argv[1], "wheel-id") == 0) {
    wheel_window(display, parse_window(argv[2]),
                 (int)parse_long(argv[3], "x coordinate"),
                 (int)parse_long(argv[4], "y coordinate"), argv[5],
                 parse_long(argv[6], "count"));
  } else if ((argc == 6 || argc == 7) && strcmp(argv[1], "drop-id") == 0) {
    drop_file(display, parse_window(argv[2]),
              (int)parse_long(argv[3], "x coordinate"),
              (int)parse_long(argv[4], "y coordinate"), argv[5],
              argc == 7 ? parse_long(argv[6], "timeout") : 5000);
  } else if (argc == 3 && strcmp(argv[1], "pid-id") == 0) {
    printf("%ld\n", window_pid(display, parse_window(argv[2])));
  } else if (argc == 2 && strcmp(argv[1], "list-windows") == 0) {
    list_windows_recursive(display, DefaultRootWindow(display));
  } else {
    print_usage(argv[0]);
    status = 2;
  }

  XCloseDisplay(display);
  return status;
}
