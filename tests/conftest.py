import pytest

from deno_core import Runtime


@pytest.fixture(scope='session')
def snapshot():
    """Creates a default initialized startup snapshot.

    deno_core requires this, although it does not document this outside of a
    few random mentions in issues.
    """
    return Runtime(will_snapshot=True).snapshot()
