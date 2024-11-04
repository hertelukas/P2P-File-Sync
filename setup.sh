#!/usr/bin/env sh

rm -rf ./docker/node1_sync ./docker/node2_sync

mkdir ./docker/node1_sync
mkdir ./docker/node2_sync

echo "Hello, I'm old" >> ./docker/node1_sync/time.txt
echo "Hello, I'm new" >> ./docker/node2_sync/time.txt

touch -m -t  202411042000.00 ./docker/node1_sync/time.txt # Set time to 20:00
touch -m -t  202411042200.00 ./docker/node2_sync/time.txt # Set time to 22:00

echo "Some content" >> ./docker/node1_sync/foo.txt
