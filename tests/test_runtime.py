from deno_core import Runtime


def test_execute_script(capfd, snapshot):
    """Ensure a simple execute_script functions as expected."""
    rn = Runtime(startup_snapshot=snapshot)
    rn.execute_script('<init>', 'Deno.core.print("HelloWorld");')

    cap = capfd.readouterr()
    assert cap.out == 'HelloWorld'


def test_heap_limit(snapshot):
    """Ensure we can restrict, resize, and remove the limits on the V8 heap."""
    rn = Runtime(
        startup_snapshot=snapshot,
        initial_heap=0,
        maximum_heap=5 * 1024 * 1024
    )

    def at_limit(current, initial):
        rn.terminate_execution()
        return current * 2

    rn.near_heap_limit(at_limit)

    for _ in range(100000):
        rn.execute_script(
            '<init>',
            'Array(0xFFFFFFFF).fill().map(_ => String.fromCharCode('
            '33 + Math.random() * (127 - 33))).join('')'
        )
