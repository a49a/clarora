/* SPDX-License-Identifier: BSD-3-Clause
 * Clarora's package-only launcher for an isolated, bundled yt-dlp Python runtime.
 * See the repository LICENSE. No shell, PATH search, or user Python is used.
 */
#include <mach-o/dyld.h>
#include <errno.h>
#include <libgen.h>
#include <limits.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>
#include <unistd.h>
#ifndef CLARORA_PYTHON_VERSION
#error CLARORA_PYTHON_VERSION must be supplied by the package builder
#endif
int main(int argc, char **argv) {
  char raw[PATH_MAX], executable[PATH_MAX], relative[PATH_MAX], home[PATH_MAX];
  char python[PATH_MAX], bootstrap[PATH_MAX];
  uint32_t size = sizeof(raw);
  if (_NSGetExecutablePath(raw, &size) || !realpath(raw, executable)) {
    fprintf(stderr, "Cannot locate the packaged yt-dlp launcher\n");
    return 126;
  }
  int n = snprintf(relative, sizeof(relative), "%s/../../Resources/python", dirname(executable));
  if (n < 0 || (size_t)n >= sizeof(relative) || !realpath(relative, home)) {
    fprintf(stderr, "Missing packaged Python runtime\n");
    return 126;
  }
  n = snprintf(python, sizeof(python), "%s/bin/python%s", home, CLARORA_PYTHON_VERSION);
  if (n < 0 || (size_t)n >= sizeof(python)) return 126;
  n = snprintf(bootstrap, sizeof(bootstrap), "%s/yt-dlp-bootstrap.py", home);
  if (n < 0 || (size_t)n >= sizeof(bootstrap)) return 126;
  const char *clear[] = {"PYTHONPATH", "PYTHONSTARTUP", "PYTHONUSERBASE", "PYTHONINSPECT",
    "DYLD_LIBRARY_PATH", "DYLD_FRAMEWORK_PATH", "DYLD_FALLBACK_LIBRARY_PATH", "DYLD_INSERT_LIBRARIES", NULL};
  for (int i = 0; clear[i]; ++i) unsetenv(clear[i]);
  if (setenv("PYTHONHOME", home, 1) || setenv("PYTHONNOUSERSITE", "1", 1) ||
      setenv("PYTHONDONTWRITEBYTECODE", "1", 1)) return 126;
  char **arguments = calloc((size_t)argc + 5, sizeof(char *));
  if (!arguments) return 126;
  arguments[0] = python;
  arguments[1] = "-S";
  arguments[2] = "-P";
  arguments[3] = bootstrap;
  for (int i = 1; i < argc; ++i) arguments[i + 3] = argv[i];
  execv(python, arguments);
  fprintf(stderr, "Cannot start packaged Python: %s\n", strerror(errno));
  free(arguments);
  return 126;
}
