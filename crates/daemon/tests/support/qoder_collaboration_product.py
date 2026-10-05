"""Qoder entry to the shared native main-Agent delegation journey."""
import sys
from collaboration_product import check_collaboration, apply_collaboration, restore_skill, run
if __name__ == '__main__':
    run(sys.argv[1], sys.argv[2])
