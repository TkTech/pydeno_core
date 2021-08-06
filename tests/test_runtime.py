from deno_core import Runtime


def test_execute_script(capfd, snapshot):
    """Ensure a simple execute_script functions as expected."""
    rn = Runtime(startup_snapshot=snapshot)
    rn.execute_script('<init>', 'Deno.core.print("HelloWorld");')

    cap = capfd.readouterr()
    assert cap.out == 'HelloWorld'
