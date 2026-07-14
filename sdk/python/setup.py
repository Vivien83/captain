from setuptools import setup

setup(
    name="captain",
    version="0.1.0",
    description="Official Python client for the Captain Agent OS REST API",
    py_modules=["captain_sdk", "captain_client"],
    python_requires=">=3.8",
    classifiers=[
        "Programming Language :: Python :: 3",
        "License :: OSI Approved :: MIT License",
        "Operating System :: OS Independent",
    ],
)
