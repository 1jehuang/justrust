"""Retention for compiler products in a freshly created replay trial."""


def finish_successful_trial(trial, *, keep):
    if not keep:
        for product in ('libreplay.so', 'libheld.rmeta', 'libnegative.rmeta', 'libedge.rmeta'):
            (trial / product).unlink(missing_ok=True)
